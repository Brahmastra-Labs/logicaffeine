//! The engine in Verilog against the machine it runs.
//!
//! `spatial::decode_engine::rtl::engine` prints the engine: the sequencer, the token-row store, the vector unit, the
//! array's columns and the card's memory port. These tests run a traced step on the machine
//! (`spatial::decode_engine::machine`) and hand the engine, under iverilog, the machine's store and memory before a job
//! with the job's words and the step's tables; the engine's store and memory after the job are the machine's — every
//! vector the machine holds element for element, a slot's lanes past its vector's width being no vector's, and the
//! memory byte for byte. Every vector job of the step — the norms, products, gates, convolutions with their rings,
//! rotations, cache rows in every format, the float matrices — over f32 and f16 states and f16, int8 and rotated int8
//! caches; and every block-format matrix on the array in every format the array reads, one activation word and two.
//! The 27B's and Qwen3.5-0.8B's engines elaborate under slang with every name declared before its use and every
//! connection made. The streaming engine's matvec (`rtl::streaming`) runs every matvec job of the step from the
//! machine's store and the weight image laid in stripes in the four channels' memories, the store after each job the
//! machine's. With the recurrence unit beside it (`Streaming::with_recurrence`), every lazily flushed memory's token-row
//! runs on the fast side over the flat space the four channels deal in granules, the rows of one sequence logging,
//! flushing and logging again on the memory the flush left, the store and the flat space after each job the machine's.

#![cfg(feature = "codegen")]

mod common;
#[path = "common/rtl_sim.rs"]
mod rtl_sim;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::quant::GgmlType;
use logicaffeine_compile::model::spec::{Ffn, Mixer, ModelSpec};
use logicaffeine_compile::spatial::decode_engine::machine::{self, encode, Fold, Job, Layout, Machine, Row, Store, StoreLayout, LANES};
use logicaffeine_compile::spatial::decode_engine::model::{CacheFormat, EngineNumerics, StateFormat};
use logicaffeine_compile::spatial::decode_engine::numerics::{FpUnit, Words};
use logicaffeine_compile::spatial::serving::{AttentionMath, StateFlush};
use logicaffeine_compile::spatial::decode::GgufHeader;
use logicaffeine_compile::spatial::decode_engine::program;
use logicaffeine_compile::spatial::decode_engine::physical::{Compaction, PhysicalMap};
use logicaffeine_compile::spatial::decode_engine::rtl::engine::{
    census, engine_verilog, simulation_twin, tables, tables_at, EngineParams, Tables, ENGINE_MIN_DIVIDE, FUNCTION_UNITS,
};
use logicaffeine_compile::spatial::decode_engine::rtl::streaming::StreamEntry;
use tiny_models::Tiny;
use rtl_sim::Simulator;

/// The tiny qwen35 with its attention grouped as the 27B's is.
fn grouped() -> ModelSpec {
    let mut spec = Tiny::Qwen35.wide(64);
    for layer in &mut spec.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            a.heads = 4;
            a.kv_heads = 2;
        }
    }
    spec
}

/// [`grouped`] with sixteen value heads of 32 elements on four key heads (each value row 512 weights, two K-quant
/// blocks), so a Gated-DeltaNet block's head jobs run more passes a step than the add's latency and one.
fn many_heads() -> ModelSpec {
    use logicaffeine_compile::model::spec::GatedDelta;
    let mut spec = grouped();
    for layer in &mut spec.layers {
        if let Mixer::GatedDelta(g) = &mut layer.mixer {
            *g = GatedDelta { k_heads: 4, v_heads: 16, head_dim: 32, conv_taps: g.conv_taps, gate: g.gate };
        }
    }
    spec
}

/// Where the engine's store differs from the machine's within the vectors the machine holds: a slot's lanes past its
/// vector's width are no vector's.
fn store_differs(beats: &[Vec<u8>], store: &StoreLayout, want: &Store) -> Option<String> {
    for (slot, rows) in want {
        for (r, v) in rows.iter().enumerate() {
            let at = store.beat(*slot, r) as usize;
            for (i, x) in v.iter().enumerate() {
                let beat = &beats[at + i / LANES];
                let lane = 4 * (i % LANES);
                let got = u32::from_le_bytes(beat[lane..lane + 4].try_into().unwrap());
                if got != x.to_bits() {
                    return Some(format!("{slot:?} row {r} element {i}: {:#010x}, the machine {:#010x}", got, x.to_bits()));
                }
            }
        }
    }
    None
}

fn hex_lines(words: impl Iterator<Item = String>) -> String {
    let mut s = String::new();
    for w in words {
        writeln!(s, "{w}").unwrap();
    }
    s
}

/// A run of bytes as the hex of one little-endian word, its most significant byte first.
fn hex_le(bytes: &[u8]) -> String {
    bytes.iter().rev().map(|b| format!("{b:02x}")).collect()
}

fn read_hex(path: &Path, bytes_a_word: usize) -> Vec<Vec<u8>> {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("//"))
        .map(|l| {
            let l = l.replace('_', "");
            assert_eq!(l.len(), 2 * bytes_a_word, "{}: a word of {} hex digits", path.display(), l.len());
            (0..bytes_a_word).rev().map(|i| u8::from_str_radix(&l[2 * i..2 * i + 2], 16).expect("a hex byte")).collect()
        })
        .collect()
}

/// A traced step and the engine built for it: the machine's jobs, layout and rows, the store laid out for them, the
/// machine's store and memory before each job and after the last, and the simulator.
struct World {
    dir: tempfile::TempDir,
    jobs: Vec<Job>,
    layout: Layout,
    rows: Vec<Row>,
    store: StoreLayout,
    snapshots: Vec<(Store, Vec<u8>)>,
    sim: Simulator,
}

impl World {
    /// The tiny model in the formats `format_of` names at `n`: the first sequence runs `warmup`, then the traced step
    /// runs a token of it beside three of a second sequence, so a sequence's rows see each other's state.
    fn of(n: EngineNumerics, format_of: &dyn Fn(&str) -> GgmlType, warmup: &[u32]) -> World {
        World::within(n, format_of, warmup, &|p| p)
    }

    /// [`World::of`] on the engine `grow` makes of the one the tiny model needs.
    fn within(n: EngineNumerics, format_of: &dyn Fn(&str) -> GgmlType, warmup: &[u32], grow: &dyn Fn(EngineParams) -> EngineParams) -> World {
        World::built(n, format_of, warmup, grow, &|v| v)
    }

    /// [`World::within`], the simulated engine the text `engine` makes of [`engine_verilog`]'s.
    fn built(
        n: EngineNumerics,
        format_of: &dyn Fn(&str) -> GgmlType,
        warmup: &[u32],
        grow: &dyn Fn(EngineParams) -> EngineParams,
        engine: &dyn Fn(String) -> String,
    ) -> World {
        let dir = tempfile::tempdir().unwrap();
        let spec = grouped();
        let path = dir.path().join("tiny.gguf");
        tiny_models::write_checkpoint_as(&spec, &path, format_of);
        let mut machine = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), n, 2, spec.ctx_len).unwrap();
        let (a, b) = (machine.sequence().unwrap(), machine.sequence().unwrap());
        machine.step(&[(a, warmup.to_vec())]).unwrap();
        let jobs = machine.jobs.clone();
        let layout = machine.layout.clone();
        let mut rows = Vec::new();
        let mut snapshots: Vec<(Store, Vec<u8>)> = Vec::new();
        machine
            .step_traced(&[(a, vec![60]), (b, vec![5, 9, 33])], &mut |t| {
                rows = t.rows.to_vec();
                snapshots.push((t.store.clone(), t.memory.to_vec()));
            })
            .unwrap();
        let store = StoreLayout::of(&jobs, &layout, rows.len());
        let params = grow(EngineParams::of(&layout, &store, &jobs));
        std::fs::write(dir.path().join("engine.v"), engine(engine_verilog(&params))).unwrap();
        std::fs::write(dir.path().join("clock.v"), logicaffeine_compile::spatial::decode_engine::ofs::COMPUTE_CLOCK_VERILOG).unwrap();
        let tb = lone_clocks(TB)
            .replace("@CHANNEL_DECLS@", "")
            .replace("@CHANNEL_PORTS@", "")
            .replace("@CHANNELS@", "  wire [31:0] moved = 32'd0;\n")
            .replace("@STORE_PROBE@", "")
            .replace("@STORES@", "")
            .replace("@SERVED@", "")
            .replace("@STALL@", &STALL.to_string())
            .replace("@DIVIDE@", &ENGINE_MIN_DIVIDE.to_string())
            .replace("@LINES@", &(layout.bytes as usize / 64).to_string())
            .replace("@JOBS@", &encode(&jobs).len().to_string())
            .replace("@ROWS@", &rows.len().to_string())
            .replace("@TENSORS@", &params.tensors.max(1).to_string())
            .replace("@ROPES@", &params.ropes.max(1).to_string())
            .replace("@LAYERS@", &params.layers.max(1).to_string())
            .replace("@STORE@", &params.store_beats.to_string())
            .replace("@STORE_READ@", &logicaffeine_compile::spatial::decode_engine::rtl::ram::STORE_READ_LATENCY.to_string());
        std::fs::write(dir.path().join("tb.v"), tb).unwrap();
        let sim = Simulator::build(dir.path(), &["engine.v", "clock.v", "tb.v"]);
        World { dir, jobs, layout, rows, store, snapshots, sim }
    }

    /// Jobs `from..to` on the engine from the machine's state before job `from`: what differs from the machine's state
    /// after job `to − 1`, named `what`.
    fn run(&self, from: usize, to: usize, what: &str) -> Vec<String> {
        self.run_counted(from, to, what).0
    }

    /// [`World::run`], with the cycles the testbench counted the engine busy for.
    fn run_counted(&self, from: usize, to: usize, what: &str) -> (Vec<String>, u64) {
        let t: Tables = tables(&self.layout, &self.store, &self.jobs[from..to], &self.rows);
        let at = self.dir.path().join(format!("jobs{from}-{to}"));
        std::fs::create_dir_all(&at).unwrap();
        let w = |name: &str, s: String| std::fs::write(at.join(name), s).unwrap();
        let (store_before, memory_before) = &self.snapshots[from];
        let (store_after, memory_after) = &self.snapshots[to];
        w("jobs.hex", hex_lines(t.jobs.iter().map(|x| format!("{x:016x}"))));
        w("rows.hex", hex_lines(t.rows.iter().map(|x| format!("{x:032x}"))));
        w("slots.hex", hex_lines(t.slots.iter().map(|x| format!("{x:016x}"))));
        w("tens.hex", hex_lines(t.tensors.iter().map(|x| format!("{x:032x}"))));
        w("ropes.hex", hex_lines(t.ropes.iter().map(|x| format!("{x:016x}"))));
        w("blocks.hex", hex_lines(t.blocks.iter().map(|f| f.iter().rev().map(|x| format!("{x:08x}")).collect())));
        w("store.hex", hex_lines(self.store.image(store_before).iter().map(|beat| beat.iter().rev().map(|x| format!("{x:08x}")).collect())));
        w("mem.hex", hex_lines(memory_before.chunks(64).map(hex_le)));
        let run = self.sim.run(
            &at,
            &[
                format!("+rows={}", self.rows.len()),
                format!("+jobs={}", t.jobs.len()),
                format!("+f16={}", t.state_f16 as u8),
                format!("+int8={}", t.cache_int8 as u8),
                format!("+two={}", t.two_words as u8),
                format!("+perblock={}", t.per_block as u8),
            ],
        );
        let out = String::from_utf8_lossy(&run.stdout);
        if !run.status.success() || out.contains("FAULT") || out.contains("TIMEOUT") {
            // A fault names the word the sequencer had reached: the job that word is in.
            let at_word = out.split("FAULT at word ").nth(1).and_then(|s| s.split_whitespace().next()).and_then(|s| s.parse::<usize>().ok());
            let job = at_word.map(|w| {
                let mut end = 0;
                let j = (from..to).find(|&j| {
                    end += encode(std::slice::from_ref(&self.jobs[j])).len();
                    w <= end
                });
                j.map_or(format!("past the jobs at word {w}"), |j| format!("in job {j} {:?}", self.jobs[j]))
            });
            return (vec![format!("{what}: {out}{}", job.unwrap_or_default())], cycles_of(&out));
        }
        let mut wrong = Vec::new();
        let beats = read_hex(&at.join("store.out"), 64);
        let mem: Vec<u8> = read_hex(&at.join("mem.out"), 64).concat();
        if let Some(d) = store_differs(&beats, &self.store, store_after) {
            wrong.push(format!("{what}: the store differs at {d}"));
        }
        if mem != *memory_after {
            let first = mem.iter().zip(memory_after).position(|(x, y)| x != y).unwrap_or(0);
            wrong.push(format!("{what}: the memory differs from byte {first} (line {})", first / 64));
        }
        wrong.extend(too_quiet(what, &out));
        (wrong, cycles_of(&out))
    }
}

/// The recurrent channels' report of the lines they gave back, for [`TB`].
const SERVED: &str = "    $display(\"SERVED %0d\", chan[0].nread + chan[1].nread + chan[2].nread + chan[3].nread);\n";

/// A vectored engine's count of its vector side's memory stores, for [`TB`]: the clocks its store stream runs, the beats
/// it asks and the stores it streams.
const STORE_PROBE: &str = r#"  integer ms_clocks = 0, ms_beats = 0, ms_puts = 0;
  reg ms_was = 1'b0;
  always @(posedge fclk) begin
    ms_was <= u.fast.vside.state == u.fast.vside.MS;
    if (u.fast.vside.state == u.fast.vside.MS) begin
      ms_clocks = ms_clocks + 1;
      if (u.fast.vside.ms_ask) ms_beats = ms_beats + 1;
      if (!ms_was) ms_puts = ms_puts + 1;
    end
  end
"#;
const STORES: &str = "    $display(\"STORES %0d %0d %0d\", ms_beats, ms_clocks, ms_puts);\n";

/// The beats, clocks and stores a vectored bench's [`STORE_PROBE`] counted, as `out` reports them.
fn stores_of(out: &str) -> Option<(u64, u64, u64)> {
    let mut n = out.split("STORES ").nth(1)?.split_whitespace().map(|x| x.parse::<u64>().ok());
    Some((n.next()??, n.next()??, n.next()??))
}

/// The lines a hybrid engine's recurrent channels gave back, as the testbench reports them.
fn served_of(out: &str) -> u64 {
    out.split("SERVED ").nth(1).and_then(|s| s.split_whitespace().next()).and_then(|n| n.parse().ok()).unwrap_or(0)
}

/// The cycles a testbench's `CYCLES <n>` line counts in `out`, 0 when it has none.
fn cycles_of(out: &str) -> u64 {
    out.split("CYCLES ").nth(1).and_then(|s| s.split_whitespace().next()).and_then(|n| n.parse().ok()).unwrap_or(0)
}

/// The compute cycles [`TB`] lets an engine stay busy with nothing moved — no line on the card port or a channel, no job
/// word — before it stops the run as stalled.
const STALL: u64 = 200_000;

/// A run that stayed busy with nothing moved for a quarter of [`STALL`] or more, as its testbench's `QUIET <n>` line
/// reports its longest such stretch: a slower build of the same run would be cut by the bound, so the bound is too
/// near the engine's own quiet stretches.
fn too_quiet(what: &str, out: &str) -> Option<String> {
    let quiet: u64 = out.split("QUIET ").nth(1).and_then(|s| s.split_whitespace().next()).and_then(|n| n.parse().ok())?;
    (4 * quiet >= STALL).then(|| format!("{what}: busy with nothing moved for {quiet} cycles, a quarter of the stall bound {STALL} or more"))
}

/// The step's jobs `runs` picks on the engine, each from the machine's state before it, the checkpoint's matrices in
/// the formats `format_of` names: the cases whose store or memory after the job differ from the machine's, named.
fn jobs_on_the_engine(n: EngineNumerics, format_of: &dyn Fn(&str) -> GgmlType, runs: &dyn Fn(&Job) -> bool) -> Vec<String> {
    jobs_on_the_engine_after(n, format_of, runs, &[3, 17, 41, 8, 29])
}

/// [`jobs_on_the_engine`], the first sequence having run `warmup` before the traced step.
fn jobs_on_the_engine_after(n: EngineNumerics, format_of: &dyn Fn(&str) -> GgmlType, runs: &dyn Fn(&Job) -> bool, warmup: &[u32]) -> Vec<String> {
    let world = World::of(n, format_of, warmup);
    let picked: Vec<usize> = (0..world.jobs.len()).filter(|&j| runs(&world.jobs[j])).collect();
    assert!(!picked.is_empty());
    picked.into_iter().flat_map(|j| world.run(j, j + 1, &format!("job {j} {:?}", world.jobs[j]))).collect()
}

/// The whole step on the engine after the host's embedding: every job from the second to the last in one run, the
/// store and the memory at its end the machine's.
fn step_on_the_engine(n: EngineNumerics, format_of: &dyn Fn(&str) -> GgmlType, warmup: &[u32]) -> Vec<String> {
    let world = World::of(n, format_of, warmup);
    assert!(matches!(world.jobs[0], Job::Embed { .. }), "the step opens with the host's embedding");
    world.run(1, world.jobs.len(), "the step")
}

/// The fastest point over `state` and `cache`, its attention on the vector units over an f16 cache.
fn point(state: StateFormat, cache: CacheFormat) -> EngineNumerics {
    let attention = if cache.is_int8() { AttentionMath::Array } else { AttentionMath::Vector };
    EngineNumerics { state, cache, attention, ..EngineNumerics::fastest() }
}

/// Every matrix a block format, the norms and the small tensors f32: the layout a K-quant mixture keeps.
fn matrices_in(ty: GgmlType) -> impl Fn(&str) -> GgmlType {
    move |name: &str| {
        let matrix = name.ends_with(".weight")
            && ["token_embd", "output.weight", "attn_q.", "attn_k.", "attn_v.", "attn_output", "attn_qkv", "attn_gate", "ssm_out", "ffn_gate.", "ffn_up.", "ffn_down."]
                .iter()
                .any(|p| name.contains(p));
        if matrix {
            ty
        } else {
            GgmlType::F32
        }
    }
}

fn vector(job: &Job) -> bool {
    matches!(job, Job::Vector(_))
}

fn matvec(job: &Job) -> bool {
    matches!(job, Job::Matvec { .. })
}

/// The engine parameters for the checkpoint a header ledger in `benchmarks/silicon/decode/headers/` describes, at the
/// fastest point, one sequence over a context of at most 4096 positions.
fn ledger_params(ledger: &str) -> EngineParams {
    ledger_params_at(ledger, EngineNumerics::fastest())
}

/// [`ledger_params`] at the knob point `n`.
fn ledger_params_at(ledger: &str, n: EngineNumerics) -> EngineParams {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/decode/headers").join(ledger);
    let header = GgufHeader::read_ledger(&path).unwrap();
    let spec = header.spec().unwrap();
    let program = program::lower(&spec, n, spec.ctx_len).unwrap().program;
    let layout = Layout::of(&spec, n, &program, &header, 1, spec.ctx_len.min(4096)).unwrap();
    let jobs = machine::lower(&program, &layout, spec.rms_eps as f32);
    let store = StoreLayout::of(&jobs, &layout, 1);
    EngineParams::of(&layout, &store, &jobs)
}

/// The engine [`ledger_params`] names, in Verilog.
fn ledger_engine(ledger: &str) -> String {
    engine_verilog(&ledger_params(ledger))
}

/// One engine runs the 27B and Qwen3.5-0.8B: their union, either way round, is the 27B's engine — its every table and
/// memory at least the 0.8B's, the 0.8B's one block format among its four — and engines whose units differ in shape
/// have none: another recurrence head width, another attention head width or math, another count of function units.
#[test]
fn one_engine_runs_the_27b_and_the_0_8b() {
    let (big, small) = (ledger_params("qwen3.8-27b-q4_k_m.json"), ledger_params("qwen3.5-0.8b-q8_0.json"));
    assert_eq!(EngineParams::union(&[big.clone(), small.clone()]), Ok(big.clone()));
    assert_eq!(EngineParams::union(&[small.clone(), big.clone()]), Ok(big.clone()));
    let unlike: [(&str, fn(&mut EngineParams)); 4] = [
        ("a recurrence head width", |p| p.delta = p.delta.map(|(w, t)| (w / 2, t))),
        ("an attention head width", |p| p.attend = p.attend.map(|(h, v, c, a)| (h / 2, v, c, a))),
        ("the attention math", |p| p.attend = p.attend.map(|(h, v, c, a)| (h, v, c, !a))),
        ("the function units", |p| p.function_units *= 2),
    ];
    for (what, change) in unlike {
        let mut other = small.clone();
        change(&mut other);
        assert!(EngineParams::union(&[big.clone(), other]).is_err(), "{what} differs, and no engine runs both");
    }
}

/// An engine names how its step carries its memories, and only the recurrence's head lanes replay a log a window
/// behind: the 27B's engine at each way of carrying them names that way; the one flushed a window behind is refused by
/// name before a line of it is printed on the reference engine, whose recurrence replays a whole log at once.
#[test]
fn an_engine_names_how_its_memories_are_carried_and_flushes_a_window_behind_on_head_lanes_alone() {
    use logicaffeine_compile::spatial::decode_engine::model::{FLUSH_TOKENS, TRAILING_TOKENS};
    let at = |flush| ledger_params_at("qwen3.8-27b-q4_k_m.json", EngineNumerics { flush, ..EngineNumerics::fastest() });
    let trailing = StateFlush::Trailing { tokens: TRAILING_TOKENS };
    for flush in [StateFlush::EveryPass, StateFlush::Every { tokens: FLUSH_TOKENS }, trailing] {
        assert_eq!(at(flush).delta.map(|(_, f)| f), Some(flush), "the engine of a step carried as {flush:?}");
    }
    let refused = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine_verilog(&at(trailing))));
    let why = refused.err().map(|e| e.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| e.downcast_ref::<String>().cloned()).unwrap_or_default());
    assert!(why.as_deref().is_some_and(|w| w.contains("a trailing flush runs on the recurrence's head lanes alone")), "the reference engine flushed a window behind printed, or refused for {why:?}");
}

/// Every recurrence job of the traced step on `lanes` head lanes under a trailing flush of four tokens at knob point
/// `n`: one sequence's row replays the last share of its log's first window and drops the window, the other's three
/// rows replay a share of a window that wraps the ring, the last share and the drop, and the next window's first share.
/// The store and the flat space after each job are the machine's. The lanes and their coordinator ask up to `credit` lines
/// ahead.
fn every_recurrence_job_flushed_a_window_behind(what: &str, n: EngineNumerics, divide: u32, granule: u64, lanes: usize, credit: usize) {
    let n = EngineNumerics { flush: StateFlush::Trailing { tokens: 4 }, ..n };
    let sides = Sides { granule: Some(granule), width: 2, lanes, credit, ..Sides::STREAMING };
    let h = HybridWorld::assembled(grouped(), n, &streamed_mix, [&TO_A_FLUSH, &TO_A_FLUSH[..10]], 1, divide, sides, &|v| v);
    let picked: Vec<usize> = (0..h.world.jobs.len()).filter(|&j| delta(&h.world.jobs[j])).collect();
    assert!(!picked.is_empty(), "the step's recurrence jobs");
    let mut wrong = Vec::new();
    for j in picked {
        wrong.extend(h.run(j, j + 1, None, &format!("{what} state on {lanes} lanes flushed a window behind: job {j} {:?}", h.world.jobs[j])));
    }
    assert_none(wrong);
}

/// [`every_recurrence_job_flushed_a_window_behind`] over an f16 state on four lanes at eight lines a granule.
#[test]
fn every_recurrence_job_runs_on_head_lanes_flushed_a_window_behind_as_on_the_machine() {
    every_recurrence_job_flushed_a_window_behind("f16", EngineNumerics::fastest(), ENGINE_MIN_DIVIDE, 8, 4, Sides::STREAMING.credit);
}

/// [`every_recurrence_job_flushed_a_window_behind`] over an f32 state on two lanes at sixteen lines a granule.
#[test]
fn every_recurrence_job_runs_on_two_head_lanes_flushed_a_window_behind_over_an_f32_state_as_on_the_machine() {
    every_recurrence_job_flushed_a_window_behind("f32", EngineNumerics { state: StateFormat::F32, ..EngineNumerics::fastest() }, 10, 16, 2, Sides::STREAMING.credit);
}

/// [`every_recurrence_job_flushed_a_window_behind`] over an f16 state on two lanes at eight lines a granule, the lanes and
/// their coordinator asking 128 lines ahead: the build hybrid9c's recurrence is.
#[test]
fn every_recurrence_job_runs_on_two_head_lanes_reading_far_ahead_flushed_a_window_behind_as_on_the_machine() {
    every_recurrence_job_flushed_a_window_behind("f16", EngineNumerics::fastest(), ENGINE_MIN_DIVIDE, 8, 2, 128);
}

/// [`every_recurrence_job_flushed_a_window_behind`]'s jobs, the f16 state on four lanes, the channels writing each line
/// 64 to 127 clocks after taking it: a sequence's next row starts only once the row before's writes are all on the
/// card, the lanes long done; every job and the whole step as on the machine.
#[test]
fn the_recurrence_runs_a_sequences_rows_on_head_lanes_after_each_rows_writes_land_late_as_on_the_machine() {
    let n = EngineNumerics { flush: StateFlush::Trailing { tokens: 4 }, ..EngineNumerics::fastest() };
    let sides = Sides { granule: Some(8), width: 2, lanes: 4, ..Sides::STREAMING };
    let h = HybridWorld::assembled(grouped(), n, &streamed_mix, [&TO_A_FLUSH, &TO_A_FLUSH[..10]], 1, ENGINE_MIN_DIVIDE, sides, &|v| v).late();
    let picked: Vec<usize> = (0..h.world.jobs.len()).filter(|&j| delta(&h.world.jobs[j])).collect();
    assert!(!picked.is_empty(), "the step's recurrence jobs");
    let mut wrong = Vec::new();
    for j in picked {
        wrong.extend(h.run(j, j + 1, None, &format!("writes landing late: job {j} {:?}", h.world.jobs[j])));
    }
    wrong.extend(h.run(1, h.world.jobs.len(), None, "the step, writes landing late"));
    assert_none(wrong);
}

/// The whole step with every unit on the fast side and its recurrence on four head lanes flushed a window of four
/// tokens behind, the store and the flat space at its end the machine's.
#[test]
fn the_whole_step_runs_with_every_unit_on_the_fast_side_and_its_recurrence_flushed_a_window_behind_on_four_head_lanes_as_on_the_machine() {
    let n = EngineNumerics { flush: StateFlush::Trailing { tokens: 4 }, ..EngineNumerics::fastest() };
    let sides = Sides { granule: Some(8), vectors: true, attention: true, width: 2, lanes: 4, ..Sides::STREAMING };
    let h = HybridWorld::assembled(grouped(), n, &streamed_mix, [&TO_A_FLUSH, &TO_A_FLUSH[..10]], 2, 8, sides, &|v| v);
    assert_none(h.run(1, h.world.jobs.len(), None, "the step on four lanes flushed a window behind"));
}

/// Every recurrence job of the traced step on head lanes that make its gated norm (`Streaming::with_delta_norm`, the
/// machine's gated norms folded into its recurrence jobs, [`Machine::delta_normed`]): each job's read-outs normed by the
/// block's weight, fetched once at the job's start, and gated by the SiLU of each row's gate as the lanes make them —
/// the store and the flat space after each job the machine's — flushed a window of four tokens behind on four lanes and
/// on two reading 128 lines ahead, and flushed every sixteen tokens on four lanes; f16 and f32 states.
#[test]
fn every_recurrence_job_runs_on_head_lanes_that_make_its_gated_norm_as_on_the_machine() {
    let trailing = EngineNumerics { flush: StateFlush::Trailing { tokens: 4 }, ..EngineNumerics::fastest() };
    let mut wrong = Vec::new();
    for (what, n, lanes, credit, granule) in [
        ("f16, a window behind", trailing, 4, Sides::STREAMING.credit, 8),
        ("f32, a window behind", EngineNumerics { state: StateFormat::F32, ..trailing }, 2, 128, 16),
        ("f16, every sixteen tokens", EngineNumerics::fastest(), 4, Sides::STREAMING.credit, 8),
    ] {
        let sides = Sides { granule: Some(granule), width: 2, lanes, credit, delta_norm: true, ..Sides::STREAMING };
        let h = HybridWorld::assembled(grouped(), n, &streamed_mix, [&TO_A_FLUSH, &TO_A_FLUSH[..10]], 1, ENGINE_MIN_DIVIDE, sides, &|v| v);
        let picked: Vec<usize> = (0..h.world.jobs.len()).filter(|&j| matches!(h.world.jobs[j], Job::Delta { norm: Some(_), .. })).collect();
        assert!(!picked.is_empty() && picked.len() == (0..h.world.jobs.len()).filter(|&j| delta(&h.world.jobs[j])).count(), "{what}: every recurrence job carries its norm");
        for j in picked {
            wrong.extend(h.run(j, j + 1, None, &format!("{what} on {lanes} lanes, normed: job {j} {:?}", h.world.jobs[j])));
        }
    }
    assert_none(wrong);
}

/// A build making its gated norms on its head lanes (`Streaming::with_delta_norm`) runs every recurrence job there: its
/// core reads each normed job's norm word — its weight's first line from the tensor table and its epsilon — and hands the
/// norm to the lanes with the job, whether the vector unit's side is beside the recurrence or not, for memories flushed
/// every sixteen tokens and a window behind. A step whose memories are rewritten every pass, whose recurrence the
/// core's own row engine runs and which makes no norm, is refused by name before a line is printed.
#[test]
fn a_build_making_its_gated_norms_on_head_lanes_reads_every_norm_word_and_refuses_memories_rewritten_every_pass() {
    use logicaffeine_compile::spatial::d5005::Beat as LineBeat;
    use logicaffeine_compile::spatial::decode_engine::image::ImageLayout;
    use logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet;
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::Streaming;
    let dir = tempfile::tempdir().unwrap();
    let spec = grouped();
    let path = dir.path().join("tiny.gguf");
    tiny_models::write_checkpoint_as(&spec, &path, &streamed_mix);
    let params_at = |flush: StateFlush, vectors: bool| -> EngineParams {
        let n = EngineNumerics { flush, ..EngineNumerics::fastest() };
        let machine = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), n, 1, spec.ctx_len).unwrap().delta_normed();
        assert!(machine.jobs.iter().any(|j| matches!(j, Job::Delta { norm: Some(_), .. })), "{flush:?}: the step's gated norms fold into its recurrence jobs");
        let store = StoreLayout::of(&machine.jobs, &machine.layout, 1);
        let mut formats: Vec<GgmlType> = Vec::new();
        for t in machine.layout.tensors.iter().filter(|t| t.streamed) {
            if !formats.contains(&t.ty) {
                formats.push(t.ty);
            }
        }
        let line = ImageLayout { stripe: Some(1), ..ImageLayout::d5005(LineBeat::Data64) };
        let streaming = Streaming::new(ColumnSet::new("step", &formats, n.activations), line).with_recurrence().with_recurrence_width(2);
        let streaming = match machine.layout.log_lines() {
            0 => Streaming { recurrence_lanes: 2, log_lines: 1, delta_norm: true, ..streaming },
            lines => streaming.with_recurrence_lanes(2, lines).with_delta_norm(),
        };
        let streaming = if vectors { streaming.with_vectors() } else { streaming };
        EngineParams { streaming: Some(streaming), ..EngineParams::of(&machine.layout, &store, &machine.jobs) }
    };
    let trailing = StateFlush::Trailing { tokens: 4 };
    for (what, flush, vectors) in [
        ("flushed every sixteen tokens", EngineNumerics::fastest().flush, false),
        ("flushed every sixteen tokens beside the vector unit's side", EngineNumerics::fastest().flush, true),
        ("flushed a window behind", trailing, false),
        ("flushed a window behind beside the vector unit's side", trailing, true),
    ] {
        let v = engine_verilog(&params_at(flush, vectors));
        assert!(v.contains("DL_NORM: begin mv_tensor <= jw[19:0]; dl_eps <= jw[51:20];"), "{what}: the core reads each normed job's norm word");
        assert!(v.contains("DL_WLINE: begin dl_wline <= mv_entry[31:0];"), "{what}: the core takes the weight's first line from the tensor table");
        assert!(v.contains("assign sm_gdn_job = {dl_wline, dl_eps, dl_zs, dl_norm,"), "{what}: the core hands the norm to the lanes with the job");
    }
    let every_pass = params_at(StateFlush::EveryPass, false);
    let refused = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine_verilog(&every_pass)));
    let why = refused.err().map(|e| e.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| e.downcast_ref::<String>().cloned()).unwrap_or_default());
    assert!(
        why.as_deref().is_some_and(|w| w.contains("a recurrence makes its gated norms on its head lanes, which run the memories it flushes lazily alone")),
        "a normed build of memories rewritten every pass printed, or refused for {why:?}"
    );
}

/// The whole step with every unit on the fast side, its recurrence on four head lanes a window behind and making its
/// gated norms there — no gated norm a vector job — the store and the flat space at its end the machine's whose gated
/// norms are folded into its recurrence jobs, and with the channels giving lines back in bursts.
#[test]
fn the_whole_step_runs_with_its_gated_norms_made_on_four_head_lanes_as_on_the_machine() {
    let n = EngineNumerics { flush: StateFlush::Trailing { tokens: 4 }, ..EngineNumerics::fastest() };
    let sides = Sides { granule: Some(8), vectors: true, attention: true, width: 2, lanes: 4, delta_norm: true, ..Sides::STREAMING };
    let h = HybridWorld::assembled(grouped(), n, &streamed_mix, [&TO_A_FLUSH, &TO_A_FLUSH[..10]], 2, 8, sides, &|v| v);
    let mut wrong = h.run(1, h.world.jobs.len(), None, "the step with its gated norms on four lanes");
    let h = h.bursting();
    wrong.extend(h.run(1, h.world.jobs.len(), None, "the step with its gated norms on four lanes, in bursts"));
    assert_none(wrong);
}

/// A recurrence unit walks a value head's memory a row of whole lines at a time, and the state format is the run's
/// knob: an engine with the unit beside its streaming unit prints for heads whose binary16 rows are whole lines (32,
/// 64, 128 wide) and refuses, naming the width, heads whose rows are not (16, 48, 80 wide), which it would walk past.
#[test]
fn a_recurrence_unit_refuses_heads_whose_binary16_rows_are_not_whole_lines() {
    use logicaffeine_compile::spatial::d5005::Beat as LineBeat;
    use logicaffeine_compile::spatial::decode_engine::image::ImageLayout;
    use logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet;
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::Streaming;
    let base = ledger_params("qwen3.5-0.8b-q8_0.json");
    let formats: Vec<GgmlType> = base.formats.iter().map(|&(ty, _)| ty).collect();
    let line = ImageLayout { stripe: Some(1), ..ImageLayout::d5005(LineBeat::Data64) };
    let recurrent = |width: usize| EngineParams {
        delta: base.delta.map(|(_, tokens)| (width, tokens)),
        streaming: Some(Streaming::new(ColumnSet::new("unit", &formats, Words::One), line.clone()).with_recurrence()),
        ..base.clone()
    };
    for width in [32, 64, 128] {
        assert!(engine_verilog(&recurrent(width)).contains("logos_gdn_side"), "a {width}-wide head's recurrence unit");
    }
    for width in [16, 48, 80] {
        let refused = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine_verilog(&recurrent(width))));
        let why = refused.err().map(|e| e.downcast_ref::<String>().cloned().unwrap_or_default());
        assert!(
            why.as_deref().is_some_and(|w| w.contains(&format!("{width} wide")) && w.contains("whole lines")),
            "a {width}-wide head's recurrence unit printed or refused for another reason: {why:?}"
        );
    }
}

/// An engine larger than a model needs runs its step exactly as the one it needs: the tiny model's whole step at the
/// fastest point on an engine with three times the vector rows, more store beats, job words, table entries and token
/// rows, a longer context and every block format the array reads at more blocks a row.
#[test]
fn the_step_runs_on_an_engine_larger_than_it_needs() {
    let grow = |p: EngineParams| {
        let mut big = p.clone();
        big.rows += 2;
        big.vector_rows *= 3;
        big.store_beats += 777;
        big.job_words += 333;
        big.tensors += 50;
        big.layers += 7;
        big.ropes += 1;
        big.attend = big.attend.map(|(h, v, c, a)| (h, v, c + 100, a));
        big.formats = [GgmlType::Q8_0, GgmlType::Q4K, GgmlType::Q5K, GgmlType::Q6K, GgmlType::Q4_0].iter().map(|&ty| (ty, 8)).collect();
        EngineParams::union(&[p, big]).unwrap()
    };
    let world = World::within(EngineNumerics::fastest(), &matrices_in(GgmlType::Q8_0), &[3, 17, 41, 8, 29], &grow);
    assert!(matches!(world.jobs[0], Job::Embed { .. }), "the step opens with the host's embedding");
    assert_none(world.run(1, world.jobs.len(), "the step on a larger engine"));
}


/// The 27B's engine at the fastest point, laid out from its header ledger: one activation quantizer, fed a beat a
/// cycle, serves every weight block and query head.
#[test]
fn the_27b_engine_quantizes_every_block_with_one_streaming_quantizer() {
    let held = census(&ledger_engine("qwen3.8-27b-q4_k_m.json"), "logos_engine");
    assert_eq!(held.get("logos_act_stream_ce"), Some(&1), "the 27B's engine: {held:?}");
    assert!(!held.contains_key("logos_act_wide"), "the 27B's engine quantizes a block at once: {held:?}");
}

/// The 27B's engine's vector unit shares `FUNCTION_UNITS` function units among its sixteen lanes.
#[test]
fn the_27b_engines_vector_unit_shares_its_function_units() {
    let v = ledger_engine("qwen3.8-27b-q4_k_m.json");
    let simd = v.lines().find(|l| l.trim_start().starts_with("logos_simd_ce #(")).expect("the engine's vector unit");
    assert!(simd.contains(&format!(".FU({FUNCTION_UNITS})")), "the 27B's vector unit: {simd}");
}

/// The diagnostics a strict front end refuses in the engine: an error; a net or port it makes up or leaves open — a
/// simulator resolves a name used before its declaration to the declaration, synthesis may bind it to a one-bit
/// implicit net; and the conversions the accelerator's build gate (`scripts/d5005/sv_check.py`) refuses in its own
/// files, a value cut to fewer bits, a signed and an unsigned operand compared, an index past its range.
const REFUSED_DIAGNOSTICS: &[&str] = &[
    "implicit-net",
    "unconnected-input-port",
    "unconnected-output-port",
    "empty-output-connection",
    "width-trunc",
    "port-width-trunc",
    "sign-compare",
    "range-oob",
];

/// `verilog` elaborated under slang from the module `top`, its parameters `params` (`name=value`): every error and every
/// diagnostic of `REFUSED_DIAGNOSTICS` slang finds, a line each.
fn strict_front_end(verilog: &str, top: &str, params: &[&str]) -> Vec<String> {
    let python = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/d5005/venv/bin/python");
    let check = r#"
import sys, pyslang
refused = sys.argv[2].split(",")
driver = pyslang.driver.Driver()
driver.addStandardArgs()
warn = " ".join(f"-W{w}" for w in refused)
params = " ".join(f"-G {p}" for p in sys.argv[4].split(",") if p)
assert driver.parseCommandLine(f"slang {sys.argv[1]} --top {sys.argv[3]} {params} --timescale 1ps/1ps {warn}", pyslang.driver.CommandLineOptions())
assert driver.processOptions() and driver.parseAllSources()
comp = driver.createCompilation()
for d in comp.getAllDiagnostics():
    text = pyslang.DiagnosticEngine.reportAll(comp.sourceManager, [d]).strip()
    if d.isError() or any(f"[-W{w}]" in text for w in refused):
        print(" | ".join(l.strip() for l in text.splitlines()[:3]))
"#;
    let dir = tempfile::tempdir().unwrap();
    let v = dir.path().join("design.v");
    std::fs::write(&v, verilog).unwrap();
    let o = Command::new(&python).args(["-c", check]).arg(&v).arg(REFUSED_DIAGNOSTICS.join(",")).arg(top).arg(params.join(",")).output().expect("slang");
    assert!(o.status.success(), "slang on {top}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).lines().map(str::to_string).collect()
}

/// The 27B's and Qwen3.5-0.8B's engines, every module in them, elaborate under slang (the language standard's rules
/// held strictly) with no error and none of `REFUSED_DIAGNOSTICS`: every name declared before its use, every port
/// a module has, every connection made, every width and sign explicit.
#[test]
fn the_engines_verilog_elaborates_under_a_strict_front_end() {
    if !common::require_tool("d5005-pim") {
        return;
    }
    let mut wrong = Vec::new();
    for ledger in ["qwen3.8-27b-q4_k_m.json", "qwen3.5-0.8b-q8_0.json"] {
        wrong.extend(strict_front_end(&ledger_engine(ledger), "logos_engine", &[]).into_iter().map(|l| format!("{ledger}: {l}")));
    }
    assert!(wrong.is_empty(), "{} diagnostics refused:\n{}", wrong.len(), wrong.iter().take(30).cloned().collect::<Vec<_>>().join("\n"));
}

/// The streaming matvec built for every format the 27B's Q4_K_M step streams (Q8_0, Q4_K, Q6_K and Q5_K; its Q4_0
/// matrices are the drafting block's) at the fastest point's one activation word, at the card's ten cores a channel, its
/// batch of token-rows and its widest row of blocks, elaborates under slang as the engines do: no error and none of
/// `REFUSED_DIAGNOSTICS`, every module in it.
#[test]
fn the_streaming_matvecs_verilog_elaborates_under_a_strict_front_end() {
    use logicaffeine_compile::spatial::d5005::Beat as LineBeat;
    use logicaffeine_compile::spatial::decode_engine::image::{machine_of, ImageLayout};
    use logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet;
    use logicaffeine_compile::spatial::decode_engine::rtl::streaming::streaming_verilog;
    if !common::require_tool("d5005-pim") {
        return;
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/decode/headers/qwen3.8-27b-q4_k_m.json");
    let machine = machine_of(&GgufHeader::read_ledger(&path).unwrap()).unwrap();
    let mut set: Vec<GgmlType> = Vec::new();
    for p in machine.tensors.iter().filter(|p| p.streamed) {
        if !set.contains(&p.ty) {
            set.push(p.ty);
        }
    }
    assert_eq!(set, [GgmlType::Q8_0, GgmlType::Q4K, GgmlType::Q6K, GgmlType::Q5K], "the formats the 27B's step streams");
    let v = streaming_verilog(&[ColumnSet::new("q4km", &set, Words::One)], &ImageLayout::d5005(LineBeat::Data64));
    let wrong = strict_front_end(&v, "logos_streaming_matvec_q4km", &["N=10", "T_MAX=64", "NB_MAX=68", "SA=16"]);
    assert!(wrong.is_empty(), "{} diagnostics refused:\n{}", wrong.len(), wrong.iter().take(40).cloned().collect::<Vec<_>>().join("\n"));
}

fn assert_none(wrong: Vec<String>) {
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn every_vector_job_runs_on_the_engine_as_on_the_machine_over_an_f16_state_and_a_rotated_int8_cache() {
    assert_none(jobs_on_the_engine(EngineNumerics::fastest(), &matrices_in(GgmlType::Q8_0), &vector));
}

#[test]
fn every_vector_job_runs_on_the_engine_as_on_the_machine_over_an_f32_state_and_an_f16_cache() {
    assert_none(jobs_on_the_engine(point(StateFormat::F32, CacheFormat::F16), &matrices_in(GgmlType::Q8_0), &vector));
}

#[test]
fn every_vector_job_runs_on_the_engine_as_on_the_machine_over_an_f16_state_and_an_int8_cache() {
    assert_none(jobs_on_the_engine(point(StateFormat::F16, CacheFormat::Int8), &matrices_in(GgmlType::Q8_0), &vector));
}

#[test]
fn every_vector_job_runs_on_the_engine_as_on_the_machine_over_an_f32_state_and_an_eager_flush() {
    let n = EngineNumerics { flush: StateFlush::EveryPass, ..point(StateFormat::F32, CacheFormat::Int8) };
    assert_none(jobs_on_the_engine(n, &matrices_in(GgmlType::Q8_0), &vector));
}

fn matrices_on_the_array(ty: GgmlType, words: Words) {
    let n = EngineNumerics { activations: words, ..EngineNumerics::fastest() };
    assert_none(jobs_on_the_engine(n, &matrices_in(ty), &matvec));
}

/// Every block-format matrix of the step on the array, in a legacy format of 32-element blocks and a K-quant of 256,
/// one activation word and two: the store after each is the machine's.
#[test]
fn every_q8_0_matrix_runs_on_the_engines_array_as_on_the_machine() {
    matrices_on_the_array(GgmlType::Q8_0, Words::One);
}

#[test]
fn every_q8_0_matrix_runs_on_the_engines_array_as_on_the_machine_in_two_words() {
    matrices_on_the_array(GgmlType::Q8_0, Words::Two);
}

#[test]
fn every_q4_k_matrix_runs_on_the_engines_array_as_on_the_machine() {
    matrices_on_the_array(GgmlType::Q4K, Words::One);
}

#[test]
fn every_q4_k_matrix_runs_on_the_engines_array_as_on_the_machine_in_two_words() {
    matrices_on_the_array(GgmlType::Q4K, Words::Two);
}

/// Every other block format the array reads, its matrices on the array at the fastest point.
#[test]
fn every_q4_0_matrix_runs_on_the_engines_array_as_on_the_machine() {
    matrices_on_the_array(GgmlType::Q4_0, Words::One);
}

#[test]
fn every_q4_1_matrix_runs_on_the_engines_array_as_on_the_machine() {
    matrices_on_the_array(GgmlType::Q4_1, Words::One);
}

#[test]
fn every_q5_0_matrix_runs_on_the_engines_array_as_on_the_machine() {
    matrices_on_the_array(GgmlType::Q5_0, Words::One);
}

#[test]
fn every_q5_1_matrix_runs_on_the_engines_array_as_on_the_machine() {
    matrices_on_the_array(GgmlType::Q5_1, Words::One);
}

#[test]
fn every_iq4_nl_matrix_runs_on_the_engines_array_as_on_the_machine() {
    matrices_on_the_array(GgmlType::Iq4Nl, Words::One);
}

#[test]
fn every_q2_k_matrix_runs_on_the_engines_array_as_on_the_machine() {
    matrices_on_the_array(GgmlType::Q2K, Words::One);
}

#[test]
fn every_q3_k_matrix_runs_on_the_engines_array_as_on_the_machine() {
    matrices_on_the_array(GgmlType::Q3K, Words::One);
}

#[test]
fn every_q5_k_matrix_runs_on_the_engines_array_as_on_the_machine() {
    matrices_on_the_array(GgmlType::Q5K, Words::One);
}

#[test]
fn every_q6_k_matrix_runs_on_the_engines_array_as_on_the_machine() {
    matrices_on_the_array(GgmlType::Q6K, Words::One);
}

#[test]
fn every_iq4_xs_matrix_runs_on_the_engines_array_as_on_the_machine() {
    matrices_on_the_array(GgmlType::Iq4Xs, Words::One);
}

fn delta(job: &Job) -> bool {
    matches!(job, Job::Delta { .. })
}

/// Fifteen tokens logged, so the traced step's first row fills the log and flushes it.
const TO_A_FLUSH: [u32; 15] = [3, 17, 41, 8, 29, 60, 5, 9, 33, 12, 7, 44, 21, 2, 50];

/// Every Gated-DeltaNet block's memory through the step on the recurrence unit — rewritten every token, and lazily
/// flushed, a row filling the log and replaying it while another sequence's rows log after one another — over f32 and
/// f16 states: the read-outs, the memory and the log after each job are the machine's.
#[test]
fn every_recurrence_runs_on_the_engine_as_on_the_machine_rewritten_every_token_in_f32() {
    let n = EngineNumerics { flush: StateFlush::EveryPass, ..point(StateFormat::F32, CacheFormat::Int8) };
    assert_none(jobs_on_the_engine_after(n, &matrices_in(GgmlType::Q8_0), &delta, &TO_A_FLUSH));
}

#[test]
fn every_recurrence_runs_on_the_engine_as_on_the_machine_rewritten_every_token_in_f16() {
    let n = EngineNumerics { flush: StateFlush::EveryPass, ..point(StateFormat::F16, CacheFormat::Int8) };
    assert_none(jobs_on_the_engine_after(n, &matrices_in(GgmlType::Q8_0), &delta, &TO_A_FLUSH));
}

#[test]
fn every_recurrence_runs_on_the_engine_as_on_the_machine_flushed_lazily_in_f16() {
    assert_none(jobs_on_the_engine_after(EngineNumerics::fastest(), &matrices_in(GgmlType::Q8_0), &delta, &TO_A_FLUSH));
}

#[test]
fn every_recurrence_runs_on_the_engine_as_on_the_machine_flushed_lazily_in_f32() {
    let n = point(StateFormat::F32, CacheFormat::Int8Hadamard);
    assert_none(jobs_on_the_engine_after(n, &matrices_in(GgmlType::Q8_0), &delta, &TO_A_FLUSH));
}

fn attend(job: &Job) -> bool {
    matches!(job, Job::Attend { .. })
}

/// Every attention block's heads over their sequence's cache on the attention unit — four query heads over two cached
/// heads, the first sequence's row at its fifteenth position beside the other's three rows at their first — on the
/// vector units over binary16 and int8 caches, and on the array over int8 and rotated int8 caches in one activation
/// word and two: the attended values after each job are the machine's.
#[test]
fn every_attention_runs_on_the_engine_as_on_the_machine_on_the_vector_units_over_an_f16_cache() {
    assert_none(jobs_on_the_engine_after(point(StateFormat::F16, CacheFormat::F16), &matrices_in(GgmlType::Q8_0), &attend, &TO_A_FLUSH));
}

#[test]
fn every_attention_runs_on_the_engine_as_on_the_machine_on_the_vector_units_over_an_int8_cache() {
    let n = EngineNumerics { attention: AttentionMath::Vector, ..point(StateFormat::F16, CacheFormat::Int8) };
    assert_none(jobs_on_the_engine_after(n, &matrices_in(GgmlType::Q8_0), &attend, &TO_A_FLUSH));
}

#[test]
fn every_attention_runs_on_the_engine_as_on_the_machine_on_the_array_over_an_int8_cache() {
    assert_none(jobs_on_the_engine_after(point(StateFormat::F16, CacheFormat::Int8), &matrices_in(GgmlType::Q8_0), &attend, &TO_A_FLUSH));
}

#[test]
fn every_attention_runs_on_the_engine_as_on_the_machine_on_the_array_over_a_rotated_cache_in_two_words() {
    let n = EngineNumerics { activations: Words::Two, ..EngineNumerics::fastest() };
    assert_none(jobs_on_the_engine_after(n, &matrices_in(GgmlType::Q8_0), &attend, &TO_A_FLUSH));
}

/// The whole step on the engine, every job after the host's embedding in one run, the logits and every state and
/// cache the step leaves the machine's: at the fastest point, flushing a log; over an f32 state rewritten every token
/// and an f16 cache on the vector units in two words; and in a K-quant.
#[test]
fn the_whole_step_runs_on_the_engine_as_on_the_machine_at_the_fastest_point() {
    assert_none(step_on_the_engine(EngineNumerics::fastest(), &matrices_in(GgmlType::Q8_0), &TO_A_FLUSH));
}

#[test]
fn the_whole_step_runs_on_the_engine_as_on_the_machine_over_an_eager_f32_state_and_an_f16_cache_in_two_words() {
    let n = EngineNumerics { activations: Words::Two, flush: StateFlush::EveryPass, ..point(StateFormat::F32, CacheFormat::F16) };
    assert_none(step_on_the_engine(n, &matrices_in(GgmlType::Q8_0), &TO_A_FLUSH));
}

#[test]
fn the_whole_step_runs_on_the_engine_as_on_the_machine_in_q4_k() {
    assert_none(step_on_the_engine(EngineNumerics::fastest(), &matrices_in(GgmlType::Q4K), &TO_A_FLUSH));
}

// ── The streaming matvec ─────────────────────────────────────────────────────────────────────────────

/// The tiny qwen35's matrices spread over every format the 27B's checkpoint holds its matrices in (its step's four and
/// its drafting block's Q4_0): the Gated-DeltaNet projections Q8_0 and Q4_0, its output Q5_K, the FFN's down projection
/// Q6_K, the rest Q4_K; the norms and the small tensors f32.
fn streamed_mix(name: &str) -> GgmlType {
    match matrices_in(GgmlType::Q4K)(name) {
        GgmlType::F32 => GgmlType::F32,
        _ if name.contains("attn_qkv") => GgmlType::Q8_0,
        _ if name.contains("attn_gate") => GgmlType::Q4_0,
        _ if name.contains("ssm_out") => GgmlType::Q5K,
        _ if name.contains("ffn_down.") => GgmlType::Q6K,
        ty => ty,
    }
}

/// The engine's testbench: the engine's clock divided `@DIVIDE@` times from the user clock by the card's divider, which
/// marks the user cycles that end with its edges; the card's memory a line a request in the engine's clock, a write
/// taken at random; the tables and the store written through the host's port a beat a cycle; the step run from the
/// words `+at` to `+end`, the store read back at the rising edge after each beat's address, as the card's core reads
/// it, and the memory written out. A hybrid engine's adds each channel's memory in the user clock answering a read
/// descriptor with its lines in order at random gaps, a descriptor taken while the engine does not give the unit the
/// channels reported.
const TB: &str = r#"module tb;
  reg fclk = 0, frst_n = 0, rst = 1, start = 0;
@FCLK2_DECL@  wire clk, crst_n, fedge, fmid;
  logos_compute_clock #(.DIVIDE(@DIVIDE@)) cc(.clk(fclk), .rst_n(frst_n), .cclk(clk), .cclk_rst_n(crst_n), .fedge(fedge),
    .fmid(fmid));
  wire busy, fault;
  wire [31:0] pc;
  wire mem_rd, mem_wr;
  wire [31:0] mem_rd_line, mem_wr_line;
  wire [511:0] mem_wr_data;
  wire [63:0] mem_wr_mask;
  reg mem_rd_valid = 0;
  reg [511:0] mem_rd_data;
  reg mem_wr_ready = 0;
  reg host_wr = 0;
  reg [2:0] host_table;
  reg [31:0] host_addr, host_raddr;
  reg [511:0] host_wdata;
  wire [511:0] host_rdata;
  reg [511:0] mem [0:@LINES@-1];
  reg [63:0] t_jobs [0:@JOBS@-1];
  reg [127:0] t_rows [0:@ROWS@-1];
  reg [63:0] t_slots [0:31];
  reg [127:0] t_tens [0:@TENSORS@-1];
  reg [63:0] t_ropes [0:@ROPES@-1];
  reg [511:0] t_blocks [0:@LAYERS@-1];
  reg [511:0] t_store [0:@STORE@-1];
  reg [15:0] rows;
  reg [31:0] job_at, job_end;
  reg state_f16, cache_int8, two_words, per_block;
  integer r, f, i, n, cycles, fo, quiet, longest, seen, card_moved = 0;
  // A run over: the shell has taken the channels back, and the card's memory takes no write of the engine's.
  reg ran = 1'b0;
  reg [31:0] last_pc;
  reg [511:0] w;
  // A run stalls once the engine is busy and nothing moves — no line on the card port or a channel, no job word — for
  // STALL compute cycles: far past any job's quiet stretch, far short of the cycle cap.
  localparam STALL = @STALL@;
@CHANNEL_DECLS@  logos_engine u(.clk(clk), .rst(rst), .start(start), .job_at(job_at), .job_end(job_end), .rows(rows), .state_f16(state_f16), .cache_int8(cache_int8),
    .two_words(two_words), .per_block(per_block), .busy(busy), .fault(fault), .pc(pc),
    .host_wr(host_wr), .host_table(host_table), .host_addr(host_addr), .host_wdata(host_wdata), .host_raddr(host_raddr), .host_rdata(host_rdata),
    .mem_rd(mem_rd), .mem_rd_line(mem_rd_line), .mem_rd_valid(mem_rd_valid),
    .mem_rd_data(mem_rd_data), .mem_wr(mem_wr), .mem_wr_line(mem_wr_line), .mem_wr_data(mem_wr_data), .mem_wr_mask(mem_wr_mask),
    .mem_wr_ready(mem_wr_ready), .fclk(fclk), .frst(!frst_n), .fedge(fedge), .fmid(fmid)@CLK2@@CHANNEL_PORTS@);
@FCLK@
  initial #17 frst_n = 1;
@CHANNELS@@STORE_PROBE@  always @(posedge clk) begin
    mem_rd_valid <= 1'b0;
    mem_wr_ready <= ran ? 1'b0 : $random;
    if (mem_rd || (mem_wr && mem_wr_ready)) card_moved <= card_moved + 1;
    if (mem_rd) begin mem_rd_data <= mem[mem_rd_line]; mem_rd_valid <= 1'b1; end
    if (mem_wr && mem_wr_ready) begin
      w = mem[mem_wr_line];
      for (i = 0; i < 64; i = i + 1) if (mem_wr_mask[i]) w[8*i +: 8] = mem_wr_data[8*i +: 8];
      mem[mem_wr_line] <= w;
    end
  end
  task put(input [2:0] table_, input integer at, input [511:0] value);
    begin
      @(negedge clk); host_wr = 1'b1; host_table = table_; host_addr = at; host_wdata = value;
      @(negedge clk); host_wr = 1'b0;
    end
  endtask
  initial begin
    if (!$value$plusargs("rows=%d", r)) r = 1;
    rows = r;
    if (!$value$plusargs("f16=%d", f)) f = 0;
    state_f16 = f;
    if (!$value$plusargs("int8=%d", f)) f = 0;
    cache_int8 = f;
    if (!$value$plusargs("two=%d", f)) f = 0;
    two_words = f;
    if (!$value$plusargs("perblock=%d", f)) f = 0;
    per_block = f;
    $readmemh("mem.hex", mem);
    $readmemh("jobs.hex", t_jobs);
    $readmemh("rows.hex", t_rows);
    $readmemh("slots.hex", t_slots);
    $readmemh("tens.hex", t_tens);
    $readmemh("ropes.hex", t_ropes);
    $readmemh("blocks.hex", t_blocks);
    $readmemh("store.hex", t_store);
    @(posedge clk); @(posedge clk); rst <= 1'b0;
    if (!$value$plusargs("jobs=%d", n)) n = 0;
    if (!$value$plusargs("at=%d", f)) f = 0;
    job_at = f;
    if (!$value$plusargs("end=%d", f)) f = n;
    job_end = f;
    for (i = 0; i < n; i = i + 1) put(3'd0, i, {448'd0, t_jobs[i]});
    for (i = 0; i < r; i = i + 1) put(3'd1, i, {384'd0, t_rows[i]});
    for (i = 0; i < 32; i = i + 1) put(3'd2, i, {448'd0, t_slots[i]});
    for (i = 0; i < @TENSORS@; i = i + 1) put(3'd3, i, {384'd0, t_tens[i]});
    for (i = 0; i < @ROPES@; i = i + 1) put(3'd4, i, {448'd0, t_ropes[i]});
    for (i = 0; i < @LAYERS@; i = i + 1) put(3'd5, i, t_blocks[i]);
    for (i = 0; i < @STORE@; i = i + 1) put(3'd6, i, t_store[i]);
    @(posedge clk); start <= 1'b1;
    @(posedge clk); start <= 1'b0;
    @(posedge clk);
    cycles = 0;
    quiet = 0;
    longest = 0;
    seen = 0;
    last_pc = pc;
    while (busy && cycles < 200000000 && quiet < STALL) begin
      @(posedge clk);
      cycles = cycles + 1;
      if (card_moved + moved != seen || pc != last_pc) begin quiet = 0; seen = card_moved + moved; last_pc = pc; end
      else begin
        quiet = quiet + 1;
        if (quiet > longest) longest = quiet;
      end
    end
    ran = 1'b1;
    @(posedge clk); @(posedge clk);
    if (fault) $display("FAULT at word %0d", pc);
    if (busy && quiet >= STALL) $display("TIMEOUT: stalled, nothing moved for %0d cycles at word %0d", quiet, pc);
    else if (busy) $display("TIMEOUT");
    $display("QUIET %0d", longest);
@SERVED@@STORES@    $display("CYCLES %0d", cycles);
    fo = $fopen("store.out", "w");
    for (i = 0; i < @STORE@; i = i + 1) begin host_raddr = i; repeat (@STORE_READ@) @(posedge clk); #1; $fwrite(fo, "%h\n", host_rdata); end
    $fclose(fo);
    $writememh("mem.out", mem);
    $finish;
  end
endmodule
"#;

/// A hybrid engine's channel memories and their connections, for [`TB`].
const CHANNEL_DECLS: &str = r#"  localparam AW = @AW@, LW = @LW@, MDW = @AW@ + @LW@ + 1;
  wire [3:0] md_valid, md_ready, mr_valid, mr_ready;
  wire [4*MDW-1:0] md_data;
  wire [2047:0] mr_data;
"#;
const CHANNEL_PORTS: &str = r#",
    .sm_md_valid(md_valid), .sm_md_ready(md_ready), .sm_md_data(md_data), .sm_mr_valid(mr_valid), .sm_mr_ready(mr_ready),
    .sm_mr_data(mr_data)"#;
const CHANNELS: &str = r#"  genvar ch;
  generate
    for (ch = 0; ch < 4; ch = ch + 1) begin : chan
      reg [511:0] lines [0:@CHANNEL_LINES@-1];
      reg [AW-1:0] at = 0;
      reg [LW-1:0] left = 0;
      reg gap = 0;
      assign md_ready[ch] = left == 0;
      assign mr_valid[ch] = left != 0 && !gap;
      assign mr_data[512*ch +: 512] = lines[at];
      reg [31:0] moved = 0;
      always @(posedge fclk) begin
        gap <= ($random & 3) == 0;
        if ((md_valid[ch] && md_ready[ch]) || (mr_valid[ch] && mr_ready[ch])) moved <= moved + 32'd1;
        if (md_valid[ch] && md_ready[ch]) begin
          if (!u.own) $display("CHANNEL %0d TAKEN OUTSIDE A JOB", ch);
          at <= md_data[MDW*ch + LW +: AW]; left <= md_data[MDW*ch +: LW];
        end else if (mr_valid[ch] && mr_ready[ch]) begin at <= at + 1; left <= left - 1; end
      end
    end
  endgenerate
  wire [31:0] moved = chan[0].moved + chan[1].moved + chan[2].moved + chan[3].moved;
  initial begin
    $readmemh("../ch0.hex", chan[0].lines);
    $readmemh("../ch1.hex", chan[1].lines);
    $readmemh("../ch2.hex", chan[2].lines);
    $readmemh("../ch3.hex", chan[3].lines);
  end
"#;

/// A hybrid engine's channels with the recurrence unit beside its streaming unit, for [`TB`]: each channel's memory
/// the image's lines below the first channel line past the image, `BASE`, and past it the flat space's lines the map
/// deals it — granule g of the space, 2^`GL` lines, on channel g mod 4 —, so the fast side's port reads and writes the
/// memory the engine's port does; a write descriptor's lines taken into a queue ahead of it as the
/// shell's write FIFO takes them, written in order once it comes, its done token raised after its last. With `+late=1`
/// a line taken is written no sooner than 64 to 127 clocks later, as a channel behind a busy controller writes it.
const RECURRENT_CHANNEL_DECLS: &str = r#"  localparam AW = @AW@, LW = @LW@, MDW = @AW@ + @LW@ + 1;
  integer burst, late;
  initial if (!$value$plusargs("burst=%d", burst)) burst = 0;
  initial if (!$value$plusargs("late=%d", late)) late = 0;
  wire bursting = burst != 0;
  reg [31:0] wnow = 0;
  always @(posedge fclk) wnow <= wnow + 32'd1;
  localparam [31:0] SPACE = @SPACE@;
  localparam [AW-1:0] BASE = @BASE@;
  localparam [4:0] GL = @GL@;
  wire [3:0] md_valid, md_ready, mr_valid, mr_ready, mw_valid, mw_ready, mdone_valid, mdone_ready;
  wire [4*MDW-1:0] md_data;
  wire [2047:0] mr_data, mw_data;
"#;
const RECURRENT_CHANNEL_PORTS: &str = r#",
    .sm_md_valid(md_valid), .sm_md_ready(md_ready), .sm_md_data(md_data), .sm_mr_valid(mr_valid), .sm_mr_ready(mr_ready),
    .sm_mr_data(mr_data), .sm_mw_valid(mw_valid), .sm_mw_ready(mw_ready), .sm_mw_data(mw_data), .sm_mdone_valid(mdone_valid),
    .sm_mdone_ready(mdone_ready), .map_space(SPACE), .map_base(BASE), .map_granule_log2(GL)"#;
const RECURRENT_CHANNELS: &str = r#"  genvar ch;
  generate
    for (ch = 0; ch < 4; ch = ch + 1) begin : chan
      reg [511:0] lines [0:@CHANNEL_LINES@-1];
      reg [AW-1:0] at = 0;
      reg [LW-1:0] left = 0;
      reg wr = 0, gap = 0, done = 0;
      reg [511:0] queue [0:63];
      reg [31:0] due [0:63];
      reg [6:0] queued = 0;
      reg [5:0] head = 0, tail = 0;
      wire [31:0] local_line = {{(32-AW){1'b0}}, at - BASE};
      wire [31:0] flat = SPACE + ((((local_line >> GL) << 2) | ch) << GL) + (local_line & ((32'd1 << GL) - 32'd1));
      wire take = mw_valid[ch] && mw_ready[ch];
      wire put = left != 0 && wr && queued != 0 && !gap && (late == 0 || wnow >= due[head]);
      // With +burst=1 a read descriptor's lines go into a queue of the channel's own as fast as it takes them and come
      // back in bursts, held while few are queued and then given back to back, as the shell's read FIFOs can give them.
      reg [511:0] bq [0:63];
      reg [6:0] bn = 0;
      reg [5:0] bh = 0, bt = 0;
      reg bhold = 0;
      wire bread = bursting && left != 0 && !wr && !gap && bn != 7'd64;
      wire bpop = bursting && mr_valid[ch] && mr_ready[ch];
      // The lines the channel has given back, and every descriptor and line it has taken or given.
      reg [31:0] nread = 0, moved = 0;
      assign md_ready[ch] = left == 0 && !done && !ran;
      assign mr_valid[ch] = bursting ? bn != 0 && !bhold : left != 0 && !wr && !gap;
      assign mr_data[512*ch +: 512] = bursting ? bq[bh] : at < BASE ? lines[at] : mem[flat];
      assign mw_ready[ch] = queued != 7'd64 && !ran;
      assign mdone_valid[ch] = done;
      always @(posedge fclk) begin
        gap <= ($random & 3) == 0;
        bhold <= bursting && (bn < 7'd8 ? ($random & 7) != 0 : ($random & 31) == 0);
        bn <= bn + {6'd0, bread} - {6'd0, bpop};
        if (bpop) bh <= bh + 6'd1;
        if (mr_valid[ch] && mr_ready[ch]) nread <= nread + 32'd1;
        if ((md_valid[ch] && md_ready[ch]) || (mr_valid[ch] && mr_ready[ch]) || take || (done && mdone_ready[ch])) moved <= moved + 32'd1;
        if (done && mdone_ready[ch]) done <= 1'b0;
        if (take) begin queue[tail] <= mw_data[512*ch +: 512]; due[tail] <= wnow + 32'd64 + ($random & 63); tail <= tail + 6'd1; end
        queued <= queued + {6'd0, take} - {6'd0, put};
        if (md_valid[ch] && md_ready[ch]) begin
          if (!u.own) $display("CHANNEL %0d TAKEN OUTSIDE A JOB", ch);
          wr <= md_data[MDW*ch + LW + AW]; at <= md_data[MDW*ch + LW +: AW]; left <= md_data[MDW*ch +: LW];
        end else if (!bursting && mr_valid[ch] && mr_ready[ch]) begin at <= at + 1; left <= left - 1; end
        else if (bread) begin
          bq[bt] <= at < BASE ? lines[at] : mem[flat];
          bt <= bt + 6'd1; at <= at + 1; left <= left - 1;
        end
        else if (put) begin
          if (at < BASE) $display("CHANNEL %0d WRITES THE IMAGE", ch);
          else mem[flat] <= queue[head];
          head <= head + 6'd1; at <= at + 1; left <= left - 1;
          if (left == 1) done <= 1'b1;
        end
      end
    end
  endgenerate
  wire [31:0] moved = chan[0].moved + chan[1].moved + chan[2].moved + chan[3].moved;
  initial begin
    $readmemh("../ch0.hex", chan[0].lines);
    $readmemh("../ch1.hex", chan[1].lines);
    $readmemh("../ch2.hex", chan[2].lines);
    $readmemh("../ch3.hex", chan[3].lines);
  end
"#;

/// A traced step and the hybrid engine built for it: [`World`]'s, the streamed matrices in their image laid in stripes,
/// the memory the engine's port reads the flat space ([`Compaction::space`]) with each streamed tensor's stream entry.
/// What a hybrid world's engine builds beside its streaming unit: the recurrence unit when `granule` names the granule
/// the channels deal the flat space in, taking `width` sixteens of elements a clock on `lanes` head lanes (the side
/// alone at 1, each lane's copy of the log as long as the layout's); the vector unit's side when
/// `vectors`, holding `float_rows` rows of a float matrix's inputs at once; the attention side beside that when
/// `attention`; the streaming unit taking Q8_0 in octets when `octets`, and every block of the image in a slot of a
/// multiple of `slot` bytes.
#[derive(Clone, Copy)]
struct Sides {
    granule: Option<u64>,
    vectors: bool,
    width: usize,
    lanes: usize,
    attention: bool,
    octets: bool,
    float_rows: usize,
    slot: usize,
    /// The array's cores paired on processing elements in `fclk2`, twice `fclk`.
    pe_pairs: bool,
    /// The array issuing its held beat against two token-rows a clock (`ColumnSet::with_token_pairs`).
    token_pairs: bool,
    /// The lanes the vector unit's side's convolution path runs a conv job's rows through (`Streaming::with_conv_path`),
    /// 0 for none.
    conv_lanes: usize,
    /// The beats of channels the conv path's kernel holds a tap; 0 the step's widest convolution's.
    conv_beats: usize,
    /// An edit of the machine's jobs before its step, the engine given the edited jobs' words.
    edit: Option<fn(&mut Vec<Job>)>,
    /// An edit of the jobs the engine is given alone, the machine stepping them as they were: a job the machine cannot
    /// step (a store from a register nothing loaded), held only to the engine's refusal.
    words: Option<fn(&mut Vec<Job>)>,
    /// The passes of a job whose passes stand alone the vector unit's side runs at once.
    passes: usize,
    /// The passes of its deepest job the vector unit's side's registers hold.
    depth: usize,
    /// The flush units each head lane splits a flushing head's rows over.
    flushes: usize,
    /// The lines the recurrence's coordinator and each head lane ask ahead of those they have taken.
    credit: usize,
    /// The fold lanes the streaming unit folds a job's sums into its output slot over, the machine's step folded
    /// ([`Machine::folded`]); 0 a step and a unit that fold nothing.
    folds: usize,
    /// Whether the recurrence's head lanes make each job's gated norm, the machine's step's gated norms folded into its
    /// recurrence jobs ([`Machine::delta_normed`]).
    delta_norm: bool,
}

impl Sides {
    /// The streaming unit alone.
    const STREAMING: Sides = Sides {
        granule: None,
        vectors: false,
        width: 1,
        lanes: 1,
        attention: false,
        octets: false,
        float_rows: logicaffeine_compile::spatial::decode_engine::rtl::vector_side::FM_ROWS,
        slot: 1,
        pe_pairs: false,
        token_pairs: false,
        conv_lanes: 0,
        conv_beats: 0,
        edit: None,
        words: None,
        passes: 1,
        depth: 1,
        flushes: 1,
        credit: logicaffeine_compile::spatial::decode_engine::rtl::gdn::LANE_CREDIT,
        folds: 0,
        delta_norm: false,
    };
}

struct HybridWorld {
    world: World,
    compaction: Compaction,
    entries: Vec<StreamEntry>,
    /// Whether the channels give lines back in bursts (`+burst=1`).
    bursts: bool,
    /// Whether the channels write each line taken 64 to 127 clocks late (`+late=1`).
    late: bool,
}

impl HybridWorld {
    /// [`World::of`] on a hybrid engine whose streaming unit takes `pairs` pairs a channel at the step's activation
    /// words, its clock divided `divide` times from the unit's.
    fn of(n: EngineNumerics, format_of: &dyn Fn(&str) -> GgmlType, warmup: &[u32], pairs: u64, divide: u32) -> HybridWorld {
        HybridWorld::built(n, format_of, warmup, pairs, divide, &|v| v)
    }

    /// [`HybridWorld::of`], the simulated engine the text `engine` makes of [`engine_verilog`]'s.
    fn built(
        n: EngineNumerics,
        format_of: &dyn Fn(&str) -> GgmlType,
        warmup: &[u32],
        pairs: u64,
        divide: u32,
        engine: &dyn Fn(String) -> String,
    ) -> HybridWorld {
        HybridWorld::assembled(grouped(), n, format_of, [warmup, &[]], pairs, divide, Sides::STREAMING, engine)
    }

    /// [`HybridWorld::of`] with the recurrence unit beside the streaming unit, the flat space dealt over the channels in
    /// granules of `granule` lines past the image, the first sequence warmed to fill its log at the traced step's row of
    /// it and the second to fill its log at its second row, the third logging on the memory the flush left.
    fn recurrent(n: EngineNumerics, format_of: &dyn Fn(&str) -> GgmlType, pairs: u64, divide: u32, granule: u64, engine: &dyn Fn(String) -> String) -> HybridWorld {
        HybridWorld::recurrent_at(n, format_of, pairs, divide, granule, 1, engine)
    }

    /// [`HybridWorld::recurrent`] with the recurrence unit taking `width` sixteens of elements a clock.
    fn recurrent_at(
        n: EngineNumerics,
        format_of: &dyn Fn(&str) -> GgmlType,
        pairs: u64,
        divide: u32,
        granule: u64,
        width: usize,
        engine: &dyn Fn(String) -> String,
    ) -> HybridWorld {
        HybridWorld::assembled(grouped(), n, format_of, [&TO_A_FLUSH, &TO_A_FLUSH[..14]], pairs, divide, Sides { granule: Some(granule), width, ..Sides::STREAMING }, engine)
    }

    /// [`HybridWorld::recurrent_at`] with the recurrence unit's value heads on `lanes` head lanes, each on a fast port of
    /// its own, the ports sharing the channels through the channel mux.
    #[allow(clippy::too_many_arguments)]
    fn recurrent_on_lanes(
        n: EngineNumerics,
        format_of: &dyn Fn(&str) -> GgmlType,
        pairs: u64,
        divide: u32,
        granule: u64,
        width: usize,
        lanes: usize,
        engine: &dyn Fn(String) -> String,
    ) -> HybridWorld {
        let sides = Sides { granule: Some(granule), width, lanes, ..Sides::STREAMING };
        HybridWorld::assembled(grouped(), n, format_of, [&TO_A_FLUSH, &TO_A_FLUSH[..14]], pairs, divide, sides, engine)
    }

    /// [`HybridWorld::attended`] with the recurrence unit taking `width` sixteens of elements a clock on `lanes` head
    /// lanes: every unit on the fast side, the lanes' ports and the shared one on the channels at once.
    fn attended_on_lanes(n: EngineNumerics, format_of: &dyn Fn(&str) -> GgmlType, pairs: u64, divide: u32, granule: u64, width: usize, lanes: usize) -> HybridWorld {
        let sides = Sides { granule: Some(granule), vectors: true, attention: true, width, lanes, ..Sides::STREAMING };
        HybridWorld::assembled(grouped(), n, format_of, [&TO_A_FLUSH, &TO_A_FLUSH[..14]], pairs, divide, sides, &|v| v)
    }

    /// [`HybridWorld::recurrent`] with the vector unit's side beside the recurrence unit, on its port: every vector job
    /// in the user clock.
    fn vectored(n: EngineNumerics, format_of: &dyn Fn(&str) -> GgmlType, pairs: u64, divide: u32, granule: u64, engine: &dyn Fn(String) -> String) -> HybridWorld {
        HybridWorld::assembled(grouped(), n, format_of, [&TO_A_FLUSH, &TO_A_FLUSH[..14]], pairs, divide, Sides { granule: Some(granule), vectors: true, ..Sides::STREAMING }, engine)
    }

    /// [`HybridWorld::vectored`] at one pair a channel, its vector unit's side running a job whose passes stand alone
    /// `passes` passes at a time.
    fn vectored_in_groups(n: EngineNumerics, format_of: &dyn Fn(&str) -> GgmlType, divide: u32, granule: u64, passes: usize) -> HybridWorld {
        HybridWorld::vectored_in_groups_on(grouped(), n, format_of, divide, granule, passes)
    }

    /// [`HybridWorld::vectored_in_groups`] of the model `spec`.
    fn vectored_in_groups_on(spec: ModelSpec, n: EngineNumerics, format_of: &dyn Fn(&str) -> GgmlType, divide: u32, granule: u64, passes: usize) -> HybridWorld {
        let sides = Sides { granule: Some(granule), vectors: true, passes, ..Sides::STREAMING };
        HybridWorld::assembled(spec, n, format_of, [&TO_A_FLUSH, &TO_A_FLUSH[..14]], 1, divide, sides, &|v| v)
    }

    /// [`HybridWorld::vectored`] with the attention side beside the vector unit's side: every attention job's token-rows
    /// in the user clock.
    fn attended(n: EngineNumerics, format_of: &dyn Fn(&str) -> GgmlType, pairs: u64, divide: u32, granule: u64, engine: &dyn Fn(String) -> String) -> HybridWorld {
        HybridWorld::assembled(grouped(), n, format_of, [&TO_A_FLUSH, &TO_A_FLUSH[..14]], pairs, divide, Sides { granule: Some(granule), vectors: true, attention: true, ..Sides::STREAMING }, engine)
    }

    /// The hybrid world: the sequences warmed with `warmups`, the recurrence unit beside the streaming unit when
    /// `granule` names the granule the channels deal the flat space in, taking `width` sixteens of elements a clock, the
    /// vector unit's side beside it when `vectors`, and the attention side beside that when `attention`.
    #[allow(clippy::too_many_arguments)]
    fn assembled(
        spec: ModelSpec,
        n: EngineNumerics,
        format_of: &dyn Fn(&str) -> GgmlType,
        warmups: [&[u32]; 2],
        pairs: u64,
        divide: u32,
        sides: Sides,
        engine: &dyn Fn(String) -> String,
    ) -> HybridWorld {
        let Sides { granule, vectors, width, lanes, attention, octets, float_rows, slot, pe_pairs, token_pairs, conv_lanes, conv_beats, edit, words, passes, depth, flushes, credit, folds, delta_norm } = sides;
        use logicaffeine_compile::spatial::d5005::Beat as LineBeat;
        use logicaffeine_compile::spatial::decode_engine::image::{streamed_image_of, ImageLayout};
        use logicaffeine_compile::spatial::decode_engine::ofs::COMPUTE_CLOCK_VERILOG;
        use logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet;
        use logicaffeine_compile::spatial::decode_engine::rtl::engine::Streaming;
        use logicaffeine_compile::spatial::decode_engine::rtl::stream::{AW, LW};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tiny.gguf");
        tiny_models::write_checkpoint_as(&spec, &path, format_of);
        let machine = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), n, 2, spec.ctx_len).unwrap();
        let machine = if folds > 0 { machine.folded() } else { machine };
        let mut machine = if delta_norm { machine.delta_normed() } else { machine };
        if let Some(edit) = edit {
            edit(&mut machine.jobs);
        }
        let (a, b) = (machine.sequence().unwrap(), machine.sequence().unwrap());
        let mut warm = vec![(a, warmups[0].to_vec())];
        if !warmups[1].is_empty() {
            warm.push((b, warmups[1].to_vec()));
        }
        machine.step(&warm).unwrap();
        let mut jobs = machine.jobs.clone();
        if let Some(words) = words {
            words(&mut jobs);
        }
        let layout = machine.layout.clone();
        let mut rows = Vec::new();
        let mut snapshots: Vec<(Store, Vec<u8>)> = Vec::new();
        machine
            .step_traced(&[(a, vec![60]), (b, vec![5, 9, 33])], &mut |t| {
                rows = t.rows.to_vec();
                snapshots.push((t.store.clone(), t.memory.to_vec()));
            })
            .unwrap();
        let store = StoreLayout::of(&jobs, &layout, rows.len());
        let line = ImageLayout { stripe: Some(pairs), octets, slot: slot as u64, ..ImageLayout::d5005(LineBeat::Data64) };
        let (image, streamed) = streamed_image_of(&layout, &line).unwrap();
        let mut formats: Vec<GgmlType> = Vec::new();
        for &i in &streamed {
            let ty = layout.tensors[i].ty;
            if !formats.contains(&ty) {
                formats.push(ty);
            }
        }
        let set = ColumnSet::new("step", &formats, n.activations);
        let set = if octets { set.with_octets() } else { set };
        let set = if slot > 1 { set.with_slots(slot) } else { set };
        let set = if pe_pairs { set.with_pe_pairs() } else { set };
        let set = if token_pairs { set.with_token_pairs() } else { set };
        let fold_of = |name: &str| {
            jobs.iter()
                .find_map(|j| match j {
                    Job::Matvec { tensor, fold, .. } if layout.tensors[*tensor].name == name => Some(*fold),
                    _ => None,
                })
                .unwrap_or(Fold::Put)
        };
        let entries: Vec<StreamEntry> =
            (0..image.tensors.len()).map(|k| StreamEntry { fold: fold_of(&image.tensors[k].name), ..StreamEntry::of(&image, k, &set).unwrap() }).collect();
        let channels = image.fill(&mut Gguf::open(&path).unwrap()).unwrap();
        for (ch, bytes) in channels.iter().enumerate() {
            std::fs::write(dir.path().join(format!("ch{ch}.hex")), hex_lines(bytes.chunks(64).map(hex_le))).unwrap();
        }
        let compaction = Compaction::of(&layout);
        let streaming = Streaming::new(set, line.clone());
        let streaming = if folds > 0 { streaming.with_folds(folds) } else { streaming };
        let (streaming, channel_decls, channel_ports, channel_model, map) = match granule {
            None => (streaming, CHANNEL_DECLS, CHANNEL_PORTS, CHANNELS, (0, 0, 0)),
            Some(g) => {
                let (map, _, _) = PhysicalMap::hybrid(&layout, &line, g).unwrap();
                assert_eq!(map.tensors_end, 0, "the hybrid's space starts at its first line");
                let streaming = streaming.with_recurrence().with_recurrence_width(width).with_recurrence_lanes(lanes, layout.log_lines());
                let streaming = if flushes > 1 { streaming.with_recurrence_flushes(flushes) } else { streaming };
                let streaming = if credit != Sides::STREAMING.credit { streaming.with_recurrence_credit(credit) } else { streaming };
                let streaming = if delta_norm { streaming.with_delta_norm() } else { streaming };
                let streaming = if vectors { streaming.with_vectors().with_float_rows(float_rows).with_vector_passes(passes).with_vector_depth(depth) } else { streaming };
                // A conv path's kernel as deep as the step's widest convolution, unless the sides name its depth.
                let widest = jobs.iter().filter_map(|j| match j { Job::Vector(v) if v.ordered() => v.loads.first().map(|l| l.len.div_ceil(16)), _ => None }).max().unwrap_or(1);
                let conv_beats = if conv_beats > 0 { conv_beats } else { widest };
                let streaming = if conv_lanes > 0 { streaming.with_conv_path(conv_lanes, conv_beats) } else { streaming };
                let streaming = if attention { streaming.with_attention() } else { streaming };
                (streaming, RECURRENT_CHANNEL_DECLS, RECURRENT_CHANNEL_PORTS, RECURRENT_CHANNELS, (0, map.base, g.trailing_zeros()))
            }
        };
        let params = EngineParams { streaming: Some(streaming), ..EngineParams::of(&layout, &store, &jobs) };
        std::fs::write(dir.path().join("engine.v"), engine(engine_verilog(&params))).unwrap();
        std::fs::write(dir.path().join("clock.v"), COMPUTE_CLOCK_VERILOG).unwrap();
        // Paired cores' processing elements in fclk2, twice fclk, the two clocks' rising edges together.
        let (fclk, fclk2_decl, clk2) = if pe_pairs {
            ("  initial begin #5 fclk = 1; forever #10 fclk = ~fclk; end\n  always #5 fclk2 = ~fclk2;", "  reg fclk2 = 0;\n", ", .clk2(fclk2)")
        } else {
            ("  always #5 fclk = ~fclk;", "", "")
        };
        let tb = TB
            .replace("@FCLK@", fclk)
            .replace("@FCLK2_DECL@", fclk2_decl)
            .replace("@CLK2@", clk2)
            .replace("@CHANNEL_DECLS@", channel_decls)
            .replace("@CHANNEL_PORTS@", channel_ports)
            .replace("@CHANNELS@", channel_model)
            .replace("@SERVED@", if granule.is_some() { SERVED } else { "" })
            .replace("@STORE_PROBE@", if vectors { STORE_PROBE } else { "" })
            .replace("@STORES@", if vectors { STORES } else { "" })
            .replace("@STALL@", &STALL.to_string())
            .replace("@SPACE@", &map.0.to_string())
            .replace("@BASE@", &map.1.to_string())
            .replace("@GL@", &map.2.to_string())
            .replace("@DIVIDE@", &divide.to_string())
            .replace("@LINES@", &(compaction.space_bytes() as usize / 64).to_string())
            .replace("@CHANNEL_LINES@", &channels.iter().map(|c| c.len() / 64).max().unwrap().max(1).to_string())
            .replace("@AW@", &AW.to_string())
            .replace("@LW@", &LW.to_string())
            .replace("@JOBS@", &encode(&jobs).len().to_string())
            .replace("@ROWS@", &rows.len().to_string())
            .replace("@TENSORS@", &params.tensors.max(1).to_string())
            .replace("@ROPES@", &params.ropes.max(1).to_string())
            .replace("@LAYERS@", &params.layers.max(1).to_string())
            .replace("@STORE@", &params.store_beats.to_string())
            .replace("@STORE_READ@", &logicaffeine_compile::spatial::decode_engine::rtl::ram::STORE_READ_LATENCY.to_string());
        std::fs::write(dir.path().join("tb.v"), tb).unwrap();
        let sim = Simulator::build(dir.path(), &["engine.v", "clock.v", "tb.v"]);
        HybridWorld { world: World { dir, jobs, layout, rows, store, snapshots, sim }, compaction, entries, bursts: false, late: false }
    }

    /// The world with its recurrent channels giving lines back in bursts ([`RECURRENT_CHANNELS`]' `+burst=1`).
    fn bursting(self) -> HybridWorld {
        HybridWorld { bursts: true, ..self }
    }

    /// The world with its recurrent channels writing each line 64 to 127 clocks after taking it ([`RECURRENT_CHANNELS`]'
    /// `+late=1`).
    fn late(self) -> HybridWorld {
        HybridWorld { late: true, ..self }
    }

    /// Jobs `from..to` on the hybrid engine from the machine's state before job `from`, at `two` activation words where
    /// given: what differs from the machine's state after job `to − 1`, named `what`.
    fn run(&self, from: usize, to: usize, two: Option<bool>, what: &str) -> Vec<String> {
        self.run_counted(from, to, two, what).0
    }

    /// [`HybridWorld::run`], with the lines the recurrent channels gave back.
    fn run_served(&self, from: usize, to: usize, what: &str) -> (Vec<String>, u64) {
        let (wrong, _, served) = self.run_measured(from, to, None, what);
        (wrong, served)
    }

    /// [`HybridWorld::run`], with the cycles the testbench counted the engine busy for.
    fn run_counted(&self, from: usize, to: usize, two: Option<bool>, what: &str) -> (Vec<String>, u64) {
        let (wrong, cycles, _) = self.run_measured(from, to, two, what);
        (wrong, cycles)
    }

    /// [`HybridWorld::run`], with the vector side's memory stores its bench counted: beats, clocks and stores.
    fn run_storing(&self, from: usize, to: usize, what: &str) -> (Vec<String>, Option<(u64, u64, u64)>) {
        let (wrong, out) = self.run_out(from, to, None, what);
        (wrong, stores_of(&out))
    }

    /// [`HybridWorld::run`], with the cycles the testbench counted the engine busy for and the lines its recurrent
    /// channels gave back.
    fn run_measured(&self, from: usize, to: usize, two: Option<bool>, what: &str) -> (Vec<String>, u64, u64) {
        let (wrong, out) = self.run_out(from, to, two, what);
        (wrong, cycles_of(&out), served_of(&out))
    }

    /// [`HybridWorld::run`], with the testbench's report.
    fn run_out(&self, from: usize, to: usize, two: Option<bool>, what: &str) -> (Vec<String>, String) {
        let w = &self.world;
        let t: Tables = tables_at(&w.layout, &w.store, &w.jobs[from..to], &w.rows, &|at| self.compaction.find(at));
        let at = w.dir.path().join(format!("jobs{from}-{to}"));
        std::fs::create_dir_all(&at).unwrap();
        let put = |name: &str, s: String| std::fs::write(at.join(name), s).unwrap();
        let (store_before, memory_before) = &w.snapshots[from];
        let (store_after, memory_after) = &w.snapshots[to];
        put("jobs.hex", hex_lines(t.jobs.iter().map(|x| format!("{x:016x}"))));
        put("rows.hex", hex_lines(t.rows.iter().map(|x| format!("{x:032x}"))));
        put("slots.hex", hex_lines(t.slots.iter().map(|x| format!("{x:016x}"))));
        put("tens.hex", hex_lines(t.tensors.iter().map(|x| format!("{x:032x}"))));
        put("ropes.hex", hex_lines(t.ropes.iter().map(|x| format!("{x:016x}"))));
        put("blocks.hex", hex_lines(t.blocks.iter().map(|f| f.iter().rev().map(|x| format!("{x:08x}")).collect())));
        put("store.hex", hex_lines(w.store.image(store_before).iter().map(|beat| beat.iter().rev().map(|x| format!("{x:08x}")).collect())));
        put("mem.hex", hex_lines(self.compaction.space(memory_before, &self.entries).chunks(64).map(hex_le)));
        let run = w.sim.run(
            &at,
            &[
                format!("+rows={}", w.rows.len()),
                format!("+jobs={}", t.jobs.len()),
                format!("+f16={}", t.state_f16 as u8),
                format!("+int8={}", t.cache_int8 as u8),
                format!("+two={}", two.unwrap_or(t.two_words) as u8),
                format!("+perblock={}", t.per_block as u8),
                format!("+burst={}", self.bursts as u8),
                format!("+late={}", self.late as u8),
            ],
        );
        let out = String::from_utf8_lossy(&run.stdout);
        if !run.status.success() || ["FAULT", "TIMEOUT", "OUTSIDE", "WRITES THE IMAGE"].iter().any(|s| out.contains(s)) {
            return (vec![format!("{what}: {out}{}", String::from_utf8_lossy(&run.stderr))], out.to_string());
        }
        let mut wrong = Vec::new();
        let beats = read_hex(&at.join("store.out"), 64);
        let mem: Vec<u8> = read_hex(&at.join("mem.out"), 64).concat();
        if let Some(d) = store_differs(&beats, &w.store, store_after) {
            wrong.push(format!("{what}: the store differs at {d}"));
        }
        let want = self.compaction.space(memory_after, &self.entries);
        if mem != want {
            let first = mem.iter().zip(&want).position(|(x, y)| x != y).unwrap_or(0);
            wrong.push(format!("{what}: the flat space differs from byte {first} (line {})", first / 64));
        }
        wrong.extend(too_quiet(what, &out));
        (wrong, out.to_string())
    }
}

/// A run that ends in writes is busy until every one of them is on the card: the shell takes its channels back once the
/// engine is idle (the step program's profile mode runs a step in pieces, each a run of its own), so the bench's memory
/// takes no write once a run is over. The convolution's job alone — its ring rows put into memory as its last work — on
/// the reference engine and on the hybrid, each leaving the memory and the store the machine's.
#[test]
fn a_run_is_busy_until_every_write_it_made_is_on_the_card() {
    let ring = |jobs: &[Job]| {
        (0..jobs.len())
            .find(|&j| matches!(&jobs[j], Job::Vector(v) if v.puts.iter().any(|p| matches!(p.to, machine::Sink::Ring { .. }))))
            .expect("a job that puts ring rows")
    };
    let w = World::of(EngineNumerics::fastest(), &matrices_in(GgmlType::Q8_0), &[3, 17]);
    let j = ring(&w.jobs);
    let mut wrong = w.run(j, j + 1, &format!("the reference engine: job {j}"));
    let h = HybridWorld::of(EngineNumerics::fastest(), &matrices_in(GgmlType::Q8_0), &[3, 17], 1, ENGINE_MIN_DIVIDE);
    let j = ring(&h.world.jobs);
    wrong.extend(h.run(j, j + 1, None, &format!("the hybrid engine: job {j}")));
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Every matvec job of the tiny model's traced step on the hybrid engine, its matrices in the five formats the 27B's
/// checkpoint holds them in streamed from their image in stripes: the store and the flat space after each job the
/// machine's — one pair a channel and one activation word with the engine's clock the fewest fast cycles a hybrid
/// engine's may be, three pairs and two words at a tenth.
#[test]
fn every_matvec_job_runs_on_the_hybrid_engine_as_on_the_machine() {
    let two = EngineNumerics { activations: Words::Two, ..EngineNumerics::fastest() };
    let mut wrong = Vec::new();
    for (n, pairs, divide) in [(EngineNumerics::fastest(), 1, ENGINE_MIN_DIVIDE), (two, 3, 10)] {
        let h = HybridWorld::of(n, &streamed_mix, &[3, 17, 41, 8, 29], pairs, divide);
        let picked: Vec<usize> = (0..h.world.jobs.len()).filter(|&j| matvec(&h.world.jobs[j])).collect();
        assert!(picked.len() > 8, "{} matvec jobs", picked.len());
        for j in picked {
            wrong.extend(h.run(j, j + 1, None, &format!("{pairs} pairs, divide {divide}: job {j} {:?}", h.world.jobs[j])));
        }
    }
    assert_none(wrong);
}

/// [`every_matvec_job_runs_on_the_hybrid_engine_as_on_the_machine`] with the streaming unit's cores paired on processing
/// elements in a clock of twice theirs (`fclk2`, its rising edges on `fclk`'s and between them): every matvec job the
/// machine's, one pair a channel (pairs across channels) at the fewest cycles an engine's may be, and three pairs and two
/// words at a tenth.
#[test]
fn every_matvec_job_runs_on_the_hybrid_engine_with_paired_cores_as_on_the_machine() {
    let two = EngineNumerics { activations: Words::Two, ..EngineNumerics::fastest() };
    let mut wrong = Vec::new();
    for (n, pairs, divide) in [(EngineNumerics::fastest(), 1, ENGINE_MIN_DIVIDE), (two, 3, 10)] {
        let h = HybridWorld::assembled(grouped(), n, &streamed_mix, [&[3, 17, 41, 8, 29], &[]], pairs, divide, Sides { pe_pairs: true, ..Sides::STREAMING }, &|v| v);
        let picked: Vec<usize> = (0..h.world.jobs.len()).filter(|&j| matvec(&h.world.jobs[j])).collect();
        assert!(picked.len() > 8, "{} matvec jobs", picked.len());
        for j in picked {
            wrong.extend(h.run(j, j + 1, None, &format!("paired cores, {pairs} pairs, divide {divide}: job {j} {:?}", h.world.jobs[j])));
        }
    }
    assert_none(wrong);
}

/// [`every_matvec_job_runs_on_the_hybrid_engine_as_on_the_machine`] with the streaming unit taking Q8_0 in octets and
/// the image laid in them: each Q8_0 matrix's eight blocks of a row an issue, into the lane a clock — every matvec job
/// the machine's, one pair at the fewest cycles an engine's may be and three pairs and two words at a tenth.
#[test]
fn every_matvec_job_runs_on_the_hybrid_engine_with_q8_0_in_octets_as_on_the_machine() {
    let two = EngineNumerics { activations: Words::Two, ..EngineNumerics::fastest() };
    let mut wrong = Vec::new();
    for (n, pairs, divide) in [(EngineNumerics::fastest(), 1, ENGINE_MIN_DIVIDE), (two, 3, 10)] {
        let h = HybridWorld::assembled(grouped(), n, &streamed_mix, [&[3, 17, 41, 8, 29], &[]], pairs, divide, Sides { octets: true, ..Sides::STREAMING }, &|v| v);
        let picked: Vec<usize> = (0..h.world.jobs.len()).filter(|&j| matvec(&h.world.jobs[j])).collect();
        assert!(picked.len() > 8, "{} matvec jobs", picked.len());
        for j in picked {
            wrong.extend(h.run(j, j + 1, None, &format!("octets, {pairs} pairs, divide {divide}: job {j} {:?}", h.world.jobs[j])));
        }
    }
    assert_none(wrong);
}

/// [`every_matvec_job_runs_on_the_hybrid_engine_as_on_the_machine`] with every block of the image in a slot of 16 bytes
/// and the streaming unit's feeds gathering its beats a slot at a time: Q8_0 in octets (272 bytes, no pad) at one pair
/// and the fewest cycles an engine's may be, and plain (34 bytes, 14 of pad) at three pairs, two words and a tenth,
/// Q6_K's 210 padded to 224 and Q4_K's 144 whole in both — every matvec job the machine's.
#[test]
fn every_matvec_job_runs_on_the_hybrid_engine_with_blocks_in_slots_as_on_the_machine() {
    let two = EngineNumerics { activations: Words::Two, ..EngineNumerics::fastest() };
    let mut wrong = Vec::new();
    for (n, pairs, divide, octets) in [(EngineNumerics::fastest(), 1, ENGINE_MIN_DIVIDE, true), (two, 3, 10, false)] {
        let h = HybridWorld::assembled(grouped(), n, &streamed_mix, [&[3, 17, 41, 8, 29], &[]], pairs, divide, Sides { octets, slot: 16, ..Sides::STREAMING }, &|v| v);
        let picked: Vec<usize> = (0..h.world.jobs.len()).filter(|&j| matvec(&h.world.jobs[j])).collect();
        assert!(picked.len() > 8, "{} matvec jobs", picked.len());
        for j in picked {
            wrong.extend(h.run(j, j + 1, None, &format!("slots of 16, octets {octets}, {pairs} pairs, divide {divide}: job {j} {:?}", h.world.jobs[j])));
        }
    }
    assert_none(wrong);
}

/// The whole step with every unit on the fast side, Q8_0 in octets and every block in a slot of 16 bytes — the card's
/// packing — the store and the flat space at its end the machine's.
#[test]
fn the_whole_step_runs_with_every_unit_on_the_fast_side_in_octets_and_slots_as_on_the_machine() {
    let h = HybridWorld::assembled(grouped(), EngineNumerics::fastest(), &streamed_mix, [&TO_A_FLUSH, &TO_A_FLUSH[..14]], 2, 8, Sides { granule: Some(8), vectors: true, attention: true, octets: true, slot: 16, ..Sides::STREAMING }, &|v| v);
    assert_none(h.run(1, h.world.jobs.len(), None, "the step in octets and slots"));
}

/// [`every_matvec_job_runs_on_the_hybrid_engine_as_on_the_machine`] on a step whose residual adds and gated products are
/// folded into its matrix jobs ([`Machine::folded`]), the streaming unit folding over `folds` lanes at `pairs` pairs and
/// the engine's clock divided `divide` times: each folding job reads every beat of its output slot it writes, folds each
/// sum into its element and writes the beat back, and every matvec job is the machine's.
fn folded_matvec_jobs_run_as_on_the_machine(n: EngineNumerics, pairs: u64, divide: u32, folds: usize) {
    folded_matvec_jobs_run_on(n, pairs, divide, Sides { folds, ..Sides::STREAMING });
}

/// [`folded_matvec_jobs_run_as_on_the_machine`] on the streaming unit `sides` build.
fn folded_matvec_jobs_run_on(n: EngineNumerics, pairs: u64, divide: u32, sides: Sides) {
    let folds = sides.folds;
    let h = HybridWorld::assembled(grouped(), n, &streamed_mix, [&[3, 17, 41, 8, 29], &[]], pairs, divide, sides, &|v| v);
    if sides.token_pairs {
        let engine = std::fs::read_to_string(h.world.dir.path().join("engine.v")).unwrap();
        assert!(engine.contains(".o_val1_b(o_val1_b), .o_val2_b(o_val2_b),"), "a streaming unit of token pairs takes its array's second token-rows' sums");
    }
    let picked: Vec<usize> = (0..h.world.jobs.len()).filter(|&j| matvec(&h.world.jobs[j])).collect();
    let folding = picked.iter().filter(|&&j| !matches!(h.world.jobs[j], Job::Matvec { fold: Fold::Put, .. })).count();
    assert!(picked.len() > 8 && folding > 4, "{} matvec jobs, {folding} of them folding", picked.len());
    let mut wrong = Vec::new();
    for j in picked {
        wrong.extend(h.run(j, j + 1, None, &format!("{folds} fold lanes, {pairs} pairs, divide {divide}: job {j} {:?}", h.world.jobs[j])));
    }
    assert_none(wrong);
}

/// [`folded_matvec_jobs_run_as_on_the_machine`] over four fold lanes, a beat each four clocks, at one pair and the fewest
/// cycles an engine's may be.
#[test]
fn every_matvec_job_of_a_folded_step_runs_on_the_hybrid_engine_as_on_the_machine_over_four_lanes() {
    folded_matvec_jobs_run_as_on_the_machine(EngineNumerics::fastest(), 1, ENGINE_MIN_DIVIDE, 4);
}

/// [`folded_matvec_jobs_run_as_on_the_machine`] over one fold lane, a beat each sixteen clocks, at three pairs (a stripe
/// a beat and a half, so stripes share beats), two activation words and a tenth.
#[test]
fn every_matvec_job_of_a_folded_step_runs_on_the_hybrid_engine_as_on_the_machine_over_one_lane_in_two_words() {
    folded_matvec_jobs_run_as_on_the_machine(EngineNumerics { activations: Words::Two, ..EngineNumerics::fastest() }, 3, 10, 1);
}

/// [`folded_matvec_jobs_run_as_on_the_machine`] over sixteen fold lanes, a beat a clock, at two pairs and an eighth.
#[test]
fn every_matvec_job_of_a_folded_step_runs_on_the_hybrid_engine_as_on_the_machine_over_sixteen_lanes() {
    folded_matvec_jobs_run_as_on_the_machine(EngineNumerics::fastest(), 2, 8, 16);
}

/// Every matvec job of a folded step runs on a hybrid engine whose array issues token pairs (`ColumnSet::with_token_pairs`)
/// as on the machine: each folding job's beats of a pair's two token-rows written one after the other, each folded into
/// its element — over four fold lanes and sixteen, at two pairs and one.
#[test]
fn every_matvec_job_of_a_folded_step_runs_on_a_hybrid_engine_of_token_pairs_as_on_the_machine() {
    folded_matvec_jobs_run_on(EngineNumerics::fastest(), 2, 8, Sides { folds: 4, token_pairs: true, ..Sides::STREAMING });
    folded_matvec_jobs_run_on(EngineNumerics::fastest(), 1, ENGINE_MIN_DIVIDE, Sides { folds: 16, token_pairs: true, ..Sides::STREAMING });
}

/// The whole folded step with every unit on the fast side, Q8_0 in octets and every block in a slot of 16 bytes, its array
/// issuing token pairs on paired processing elements in clk2 (the card's packing with token pairs): the store and the flat
/// space at its end the machine's.
#[test]
fn the_whole_folded_step_runs_with_every_unit_on_the_fast_side_on_token_pairs_as_on_the_machine() {
    let sides = Sides { granule: Some(8), vectors: true, attention: true, octets: true, slot: 16, folds: 4, pe_pairs: true, token_pairs: true, ..Sides::STREAMING };
    let h = HybridWorld::assembled(grouped(), EngineNumerics::fastest(), &streamed_mix, [&TO_A_FLUSH, &TO_A_FLUSH[..14]], 2, 8, sides, &|v| v);
    let engine = std::fs::read_to_string(h.world.dir.path().join("engine.v")).unwrap();
    assert!(engine.contains("logos_matvec_pass_x2t #("), "the array's cores run their token pairs on elements in clk2");
    assert_none(h.run(1, h.world.jobs.len(), None, "the folded step on token pairs"));
}

/// The whole folded step with every unit on the fast side, Q8_0 in octets and every block in a slot of 16 bytes, the
/// streaming unit folding over four lanes: the store and the flat space at its end the machine's.
#[test]
fn the_whole_folded_step_runs_with_every_unit_on_the_fast_side_as_on_the_machine() {
    let sides = Sides { granule: Some(8), vectors: true, attention: true, octets: true, slot: 16, folds: 4, ..Sides::STREAMING };
    let h = HybridWorld::assembled(grouped(), EngineNumerics::fastest(), &streamed_mix, [&TO_A_FLUSH, &TO_A_FLUSH[..14]], 2, 8, sides, &|v| v);
    assert!(h.world.jobs.iter().any(|j| matches!(j, Job::Matvec { fold: Fold::SwiGlu, .. })), "the step folds its gated products");
    assert_none(h.run(1, h.world.jobs.len(), None, "the folded step"));
}

/// A vector job's loads wait for the card only where they read what the job wrote there: a convolution's ring load of a
/// row whose sequence stored a ring row earlier in the job. Over the traced step's rows — one sequence's one, then
/// another's three — every convolution job is the machine's, and the convolutions take fewer cycles than on a side whose
/// every load of every pass waits for every write before it (its former wait, put back in its text), and fewer than on
/// one whose every ring load waits once any ring row is stored (the sequence's region left out of the test), so the
/// second sequence's first row reads its ring at once.
#[test]
fn a_convolution_waits_for_the_card_only_for_its_own_sequences_ring() {
    let edit = |from: &'static str, to: &'static str| {
        move |v: String| {
            assert_eq!(v.matches(from).count(), 1, "the side's `{from}`");
            v.replacen(from, to, 1)
        }
    };
    let every_load = edit("else if (!ring_waits || settled) begin", "else if (settled) begin");
    let every_ring = edit(
        "wire ring_waits = ld_a[li][7:4] == 4'd3 && ring_open && ring_region == row_region;",
        "wire ring_waits = ld_a[li][7:4] == 4'd3 && ring_open;",
    );
    let world = |engine: &dyn Fn(String) -> String| HybridWorld::vectored(EngineNumerics::fastest(), &streamed_mix, 1, 8, 8, engine);
    let worlds = [("as built", world(&|v| v)), ("every load waiting", world(&every_load)), ("every ring load waiting", world(&every_ring))];
    let convs: Vec<usize> = (0..worlds[0].1.world.jobs.len()).filter(|&j| matches!(&worlds[0].1.world.jobs[j], Job::Vector(v) if v.ordered())).collect();
    assert!(!convs.is_empty(), "the step convolves");
    let mut wrong = Vec::new();
    let cycles: Vec<u64> = worlds
        .iter()
        .map(|(what, h)| {
            convs
                .iter()
                .map(|&j| {
                    let (w, c) = h.run_counted(j, j + 1, None, &format!("convolution job {j}, {what}"));
                    wrong.extend(w);
                    c
                })
                .sum()
        })
        .collect();
    assert_none(wrong);
    assert!(cycles[0] < cycles[1] && cycles[0] < cycles[2], "the convolutions in {} cycles, against {} with every load waiting and {} with every ring load", cycles[0], cycles[1], cycles[2]);
}

/// Every convolution job of the traced step — one sequence's row, then another's three, so a sequence's next row reads
/// the ring row its last wrote — runs on the vector unit's side's conv path (`Streaming::with_conv_path`) exactly as on
/// the machine: the kernel read once a job, each row's three ring rows and its x streamed through the conv lanes, its
/// output written into its slot and x into the ring's oldest row; on one, four and sixteen lanes, and on four lanes in
/// fewer cycles than the side's micro-program takes the same jobs.
#[test]
fn every_convolution_job_runs_on_the_vector_sides_conv_path_as_on_the_machine() {
    let world = |lanes: usize| {
        let sides = Sides { granule: Some(8), vectors: true, conv_lanes: lanes, ..Sides::STREAMING };
        HybridWorld::assembled(grouped(), EngineNumerics::fastest(), &streamed_mix, [&TO_A_FLUSH, &TO_A_FLUSH[..14]], 1, 8, sides, &|v| v)
    };
    let mut wrong = Vec::new();
    let mut cycles = Vec::new();
    for lanes in [0usize, 1, 4, 16] {
        let h = world(lanes);
        let engine = std::fs::read_to_string(h.world.dir.path().join("engine.v")).unwrap();
        assert_eq!(engine.contains("logos_conv_path #("), lanes > 0, "a side of {lanes} conv lanes builds the path where it asks for one");
        let convs: Vec<usize> = (0..h.world.jobs.len()).filter(|&j| matches!(&h.world.jobs[j], Job::Vector(v) if v.ordered())).collect();
        assert!(!convs.is_empty(), "the step convolves");
        let mut total = 0u64;
        for &j in &convs {
            let (w, c) = h.run_counted(j, j + 1, None, &format!("convolution job {j}, {lanes} conv lanes"));
            wrong.extend(w);
            total += c;
        }
        cycles.push(total);
    }
    assert_none(wrong);
    assert!(cycles[2] < cycles[0], "the convolutions on four conv lanes in {} cycles, against {} on the side's micro-program", cycles[2], cycles[0]);
}

/// Every convolution job of the traced step runs on the conv path of a side behind the pipelined token-row store
/// (`logos_store_w512`, its reads `STORE_READ_LATENCY` clocks, the store of a build with every unit on the fast side) as
/// on the machine: one sequence's row, then another's three, a sequence's next row reading the ring row its last wrote.
#[test]
fn every_convolution_job_runs_on_the_conv_path_behind_the_pipelined_store_as_on_the_machine() {
    let sides = Sides { granule: Some(8), vectors: true, attention: true, conv_lanes: 4, ..Sides::STREAMING };
    let h = HybridWorld::assembled(grouped(), EngineNumerics::fastest(), &streamed_mix, [&TO_A_FLUSH, &TO_A_FLUSH[..14]], 1, 8, sides, &|v| v);
    let engine = std::fs::read_to_string(h.world.dir.path().join("engine.v")).unwrap();
    assert!(engine.contains("  logos_store_w512 #("), "a build with every unit on the fast side reads the pipelined store");
    assert!(engine.contains("logos_conv_path #("), "and runs its convolutions on the path");
    let convs: Vec<usize> = (0..h.world.jobs.len()).filter(|&j| matches!(&h.world.jobs[j], Job::Vector(v) if v.ordered())).collect();
    assert!(!convs.is_empty(), "the step convolves");
    let mut wrong = Vec::new();
    for &j in &convs {
        wrong.extend(h.run(j, j + 1, None, &format!("convolution job {j} behind the pipelined store")));
    }
    assert_none(wrong);
}

/// The first vector job of a step that runs on the vector unit — not a convolution, a program, a load and a store.
fn first_unit_job(jobs: &mut [Job]) -> &mut machine::VectorJob {
    jobs.iter_mut()
        .find_map(|j| match j {
            Job::Vector(v) if !v.ordered() && !v.program.is_empty() && !v.loads.is_empty() && !v.puts.is_empty() => Some(v),
            _ => None,
        })
        .expect("a vector job on the unit")
}

/// The vector unit of a side whose convolutions run on their path holds `CONV_PATH_VREGS` registers, half a side's
/// without one, and the side refuses a job bound for its unit whose words name a register past them: a load into one, a
/// store from one and an instruction writing one, each alone in a job the step runs on its unit, and a convolution its
/// path cannot take (its channels a beat past the path's kernel), each fault on the side with the path, where the side
/// without it runs the load's, the instruction's and the convolution's jobs as the machine does. The jobs name the
/// first register past the unit's, so a side that refused only from a later one would run them.
#[test]
fn a_conv_path_sides_vector_unit_holds_eight_registers_and_its_side_refuses_a_job_naming_a_ninth() {
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::CONV_PATH_VREGS;
    use logicaffeine_compile::spatial::decode_engine::vector::{Bin, Op};
    assert_eq!(CONV_PATH_VREGS, 8, "the unit's fields name sixteen registers, the side's half of them");
    let world = |lanes: usize, beats: usize, edit: Option<fn(&mut Vec<Job>)>, words: Option<fn(&mut Vec<Job>)>| {
        let sides = Sides { granule: Some(8), vectors: true, conv_lanes: lanes, conv_beats: beats, edit, words, ..Sides::STREAMING };
        HybridWorld::assembled(grouped(), EngineNumerics::fastest(), &streamed_mix, [&TO_A_FLUSH, &TO_A_FLUSH[..14]], 1, 8, sides, &|v| v)
    };
    let unit = |h: &HybridWorld| {
        let engine = std::fs::read_to_string(h.world.dir.path().join("engine.v")).unwrap();
        let at = engine.find("  logos_simd_p #(").expect("the side's vector unit");
        let unit = &engine[at..at + engine[at..].find(" vu(").expect("its instance")];
        unit.split(".VREGS(").nth(1).and_then(|r| r.split(')').next()).expect("its registers").to_string()
    };
    let faults = |wrong: &[String]| wrong.len() == 1 && wrong[0].contains("FAULT");
    let plain = world(0, 0, None, None);
    assert_eq!(unit(&plain), "16", "a side without the path holds sixteen registers");
    let pathed = world(4, 0, None, None);
    assert_eq!(unit(&pathed), CONV_PATH_VREGS.to_string(), "a side with the path holds {CONV_PATH_VREGS}");
    let convs: Vec<usize> = (0..plain.world.jobs.len()).filter(|&j| matches!(&plain.world.jobs[j], Job::Vector(v) if v.ordered())).collect();
    let c = convs[0];
    let widest = convs.iter().filter_map(|&j| match &plain.world.jobs[j] { Job::Vector(v) => v.loads.first().map(|l| l.len.div_ceil(16)), _ => None }).max().unwrap();
    assert!(widest > 1, "the step's convolutions span more than a beat");
    assert_none(plain.run(c, c + 1, None, &format!("convolution job {c} on a unit of sixteen registers")));
    assert_none(pathed.run(c, c + 1, None, &format!("convolution job {c} on the path")));
    let short = world(4, widest - 1, None, None);
    assert_eq!(unit(&short), CONV_PATH_VREGS.to_string());
    let wrong = short.run(c, c + 1, None, &format!("convolution job {c}, a beat past the path's kernel"));
    assert!(faults(&wrong), "a convolution the path cannot take, on a unit of {CONV_PATH_VREGS} registers: {wrong:?}");
    let load: fn(&mut Vec<Job>) = |jobs| {
        let v = first_unit_job(jobs);
        let l = v.loads[0];
        v.loads.push(machine::Load { reg: CONV_PATH_VREGS as u8, ..l });
    };
    let store: fn(&mut Vec<Job>) = |jobs| {
        let v = first_unit_job(jobs);
        let p = v.puts[0];
        v.puts.push(machine::Put { reg: machine::Reg::V(CONV_PATH_VREGS as u8), ..p });
    };
    let instruction: fn(&mut Vec<Job>) = |jobs| {
        let v = first_unit_job(jobs);
        let r = v.loads[0].reg;
        v.program.push(Op::Vec { op: Bin::Add, a: r, b: r, d: CONV_PATH_VREGS as u8 });
    };
    let j = (0..plain.world.jobs.len())
        .find(|&j| matches!(&plain.world.jobs[j], Job::Vector(v) if !v.ordered() && !v.program.is_empty() && !v.loads.is_empty() && !v.puts.is_empty()))
        .unwrap();
    // The store's job stores a register nothing loaded: the machine steps it as it was and the engine is given the
    // edited words alone, so only its refusal is held to anything.
    for (what, edit, runs) in [("a load into register 8", load, true), ("a store from register 8", store, false), ("an instruction writing register 8", instruction, true)] {
        if runs {
            assert_none(world(0, 0, Some(edit), None).run(j, j + 1, None, &format!("job {j} with {what} on a unit of sixteen registers")));
        }
        let h = if runs { world(4, 0, Some(edit), None) } else { world(4, 0, None, Some(edit)) };
        let wrong = h.run(j, j + 1, None, &format!("job {j} with {what} on a unit of {CONV_PATH_VREGS}"));
        assert!(faults(&wrong), "job {j} with {what}, on a unit of {CONV_PATH_VREGS} registers: {wrong:?}");
    }
}

/// The whole step with every unit on the fast side and its convolutions on the vector unit's side's conv path, folded
/// and in octets and slots as the card builds it: the store and the flat space at its end the machine's.
#[test]
fn the_whole_step_runs_with_its_convolutions_on_the_vector_sides_conv_path_as_on_the_machine() {
    let sides = Sides { granule: Some(8), vectors: true, attention: true, octets: true, slot: 16, folds: 4, conv_lanes: 4, ..Sides::STREAMING };
    let h = HybridWorld::assembled(grouped(), EngineNumerics::fastest(), &streamed_mix, [&TO_A_FLUSH, &TO_A_FLUSH[..14]], 2, 8, sides, &|v| v);
    let engine = std::fs::read_to_string(h.world.dir.path().join("engine.v")).unwrap();
    assert!(engine.contains("logos_conv_path #("), "the side runs its convolutions on the path");
    assert_none(h.run(1, h.world.jobs.len(), None, "the step with its convolutions on the path"));
}

/// The whole step with every unit on the fast side and Q8_0 in octets, the store and the flat space at its end the
/// machine's.
#[test]
fn the_whole_step_runs_with_every_unit_on_the_fast_side_and_q8_0_in_octets_as_on_the_machine() {
    let h = HybridWorld::assembled(grouped(), EngineNumerics::fastest(), &streamed_mix, [&TO_A_FLUSH, &TO_A_FLUSH[..14]], 2, 8, Sides { granule: Some(8), vectors: true, attention: true, octets: true, ..Sides::STREAMING }, &|v| v);
    assert_none(h.run(1, h.world.jobs.len(), None, "the step in octets"));
}

/// The whole step on the hybrid engine after the host's embedding — every vector, recurrence and attention job on the
/// reference engine's units, every matvec job on the streaming unit — the store and the flat space at its end the
/// machine's, with the engine's clock an eighth of the unit's.
#[test]
fn the_whole_step_runs_on_the_hybrid_engine_as_on_the_machine() {
    let h = HybridWorld::of(EngineNumerics::fastest(), &streamed_mix, &[3, 17, 41, 8, 29], 2, 8);
    assert!(matches!(h.world.jobs[0], Job::Embed { .. }), "the step opens with the host's embedding");
    assert_none(h.run(1, h.world.jobs.len(), None, "the step"));
}

/// Two contexts' steps on one engine, every unit on the fast side: the machine's traced step of each context's two
/// sequences, one after the other, and the engine built for them, each context a core of its own over its own four
/// token-rows and its bank of the store.
struct ContextsWorld {
    dir: tempfile::TempDir,
    jobs: Vec<Job>,
    store: StoreLayout,
    rows: [Vec<Row>; 2],
    snapshots: [Vec<(Store, Vec<u8>)>; 2],
    tables: [Tables; 2],
    compaction: Compaction,
    entries: Vec<StreamEntry>,
    sim: Simulator,
    /// Whether the channels write each line taken 64 to 127 clocks late (`+late=1`).
    late: bool,
}

/// [`TB`] for an engine of two contexts: `+rows0` and `+rows1` token-rows, each context's rows table and store written
/// with its context in the host address's top four bits, every context with rows started at once, and each bank of the
/// store read back into a file of its own.
/// `tb` with the engine's clocks as an engine of lone cores takes them: `fclk` alone.
fn lone_clocks(tb: &str) -> String {
    tb.replace("@FCLK@", "  always #5 fclk = ~fclk;").replace("@FCLK2_DECL@", "").replace("@CLK2@", "")
}

fn contexts_bench(tb: &str) -> String {
    let edits: [(&str, &str); 6] = [
        ("  reg [15:0] rows;\n", "  reg [31:0] rows;\n  integer r1;\n"),
        ("    if (!$value$plusargs(\"rows=%d\", r)) r = 1;\n    rows = r;\n", "    if (!$value$plusargs(\"rows0=%d\", r)) r = 0;\n    if (!$value$plusargs(\"rows1=%d\", r1)) r1 = 0;\n    rows = {r1[15:0], r[15:0]};\n"),
        ("    $readmemh(\"rows.hex\", t_rows);\n", "    $readmemh(\"rows0.hex\", t_rows0);\n    $readmemh(\"rows1.hex\", t_rows1);\n"),
        ("    $readmemh(\"store.hex\", t_store);\n", "    $readmemh(\"store0.hex\", t_store0);\n    $readmemh(\"store1.hex\", t_store1);\n"),
        (
            "    for (i = 0; i < r; i = i + 1) put(3'd1, i, {384'd0, t_rows[i]});\n",
            "    for (i = 0; i < r; i = i + 1) put(3'd1, i, {384'd0, t_rows0[i]});\n    for (i = 0; i < r1; i = i + 1) put(3'd1, (1 << 28) | i, {384'd0, t_rows1[i]});\n",
        ),
        (
            "    for (i = 0; i < @STORE@; i = i + 1) put(3'd6, i, t_store[i]);\n",
            "    for (i = 0; i < @STORE@; i = i + 1) put(3'd6, i, t_store0[i]);\n    for (i = 0; i < @STORE@; i = i + 1) put(3'd6, (1 << 28) | i, t_store1[i]);\n",
        ),
    ];
    let mut out = tb.to_string();
    for (from, to) in edits {
        assert_eq!(out.matches(from).count(), 1, "`{from}`");
        out = out.replace(from, to);
    }
    let tail: [(&str, &str); 4] = [
        ("  reg [127:0] t_rows [0:@ROWS@-1];\n", "  reg [127:0] t_rows0 [0:@ROWS@-1];\n  reg [127:0] t_rows1 [0:@ROWS@-1];\n"),
        ("  reg [511:0] t_store [0:@STORE@-1];\n", "  reg [511:0] t_store0 [0:@STORE@-1];\n  reg [511:0] t_store1 [0:@STORE@-1];\n"),
        ("    if (fault) $display(\"FAULT at word %0d\", pc);\n", "    if (fault) $display(\"FAULT at words %0d %0d\", pc[15:0], pc[31:16]);\n"),
        (
            "    fo = $fopen(\"store.out\", \"w\");\n    for (i = 0; i < @STORE@; i = i + 1) begin host_raddr = i; repeat (@STORE_READ@) @(posedge clk); #1; $fwrite(fo, \"%h\\n\", host_rdata); end\n    $fclose(fo);\n",
            "    fo = $fopen(\"store0.out\", \"w\");\n    for (i = 0; i < @STORE@; i = i + 1) begin host_raddr = i; repeat (@STORE_READ@) @(posedge clk); #1; $fwrite(fo, \"%h\\n\", host_rdata); end\n    $fclose(fo);\n    \
             fo = $fopen(\"store1.out\", \"w\");\n    for (i = 0; i < @STORE@; i = i + 1) begin host_raddr = (1 << 28) | i; repeat (@STORE_READ@) @(posedge clk); #1; $fwrite(fo, \"%h\\n\", host_rdata); end\n    $fclose(fo);\n",
        ),
    ];
    for (from, to) in tail {
        assert_eq!(out.matches(from).count(), 1, "`{from}`");
        out = out.replace(from, to);
    }
    out
}

impl ContextsWorld {
    /// The world of `lanes` head lanes: context 0 the sequences warmed to flush their logs at the traced step's first and
    /// second rows of them, context 1 two more warmed to flush at their second and at no row, the vector unit's side
    /// holding `float_rows` rows of a float matrix's inputs at once.
    fn of(lanes: usize, float_rows: usize) -> ContextsWorld {
        use logicaffeine_compile::spatial::d5005::Beat as LineBeat;
        use logicaffeine_compile::spatial::decode_engine::image::{streamed_image_of, ImageLayout};
        use logicaffeine_compile::spatial::decode_engine::ofs::COMPUTE_CLOCK_VERILOG;
        use logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet;
        use logicaffeine_compile::spatial::decode_engine::rtl::engine::Streaming;
        use logicaffeine_compile::spatial::decode_engine::rtl::stream::{AW, LW};
        const PAIRS: u64 = 2;
        const GRANULE: u64 = 8;
        let (spec, n) = (grouped(), EngineNumerics::fastest());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tiny.gguf");
        tiny_models::write_checkpoint_as(&spec, &path, &streamed_mix);
        let mut machine = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), n, 4, spec.ctx_len).unwrap();
        let seqs: Vec<usize> = (0..4).map(|_| machine.sequence().unwrap()).collect();
        machine
            .step(&[(seqs[0], TO_A_FLUSH.to_vec()), (seqs[1], TO_A_FLUSH[..14].to_vec()), (seqs[2], TO_A_FLUSH[..13].to_vec()), (seqs[3], vec![4, 6])])
            .unwrap();
        let jobs = machine.jobs.clone();
        let layout = machine.layout.clone();
        let mut traced = |entries: &[(usize, Vec<u32>)]| {
            let mut rows = Vec::new();
            let mut snapshots: Vec<(Store, Vec<u8>)> = Vec::new();
            machine
                .step_traced(entries, &mut |t| {
                    rows = t.rows.to_vec();
                    snapshots.push((t.store.clone(), t.memory.to_vec()));
                })
                .unwrap();
            (rows, snapshots)
        };
        let (rows0, snaps0) = traced(&[(seqs[0], vec![60]), (seqs[1], vec![5, 9, 33])]);
        let (rows1, snaps1) = traced(&[(seqs[2], vec![17, 41]), (seqs[3], vec![8, 29])]);
        assert_eq!(rows0.len(), rows1.len(), "both contexts' banks laid for one count of rows");
        assert_eq!(machine.jobs, jobs, "one program for both contexts");
        let store = StoreLayout::of(&jobs, &layout, rows0.len());
        let line = ImageLayout { stripe: Some(PAIRS), ..ImageLayout::d5005(LineBeat::Data64) };
        let (image, streamed) = streamed_image_of(&layout, &line).unwrap();
        let mut formats: Vec<GgmlType> = Vec::new();
        for &i in &streamed {
            if !formats.contains(&layout.tensors[i].ty) {
                formats.push(layout.tensors[i].ty);
            }
        }
        let set = ColumnSet::new("step", &formats, n.activations);
        let entries: Vec<StreamEntry> = (0..image.tensors.len()).map(|k| StreamEntry::of(&image, k, &set).unwrap()).collect();
        let channels = image.fill(&mut Gguf::open(&path).unwrap()).unwrap();
        for (ch, bytes) in channels.iter().enumerate() {
            std::fs::write(dir.path().join(format!("ch{ch}.hex")), hex_lines(bytes.chunks(64).map(hex_le))).unwrap();
        }
        let compaction = Compaction::of(&layout);
        let (map, _, _) = PhysicalMap::hybrid(&layout, &line, GRANULE).unwrap();
        let streaming = Streaming::new(set, line.clone())
            .with_recurrence()
            .with_recurrence_width(2)
            .with_recurrence_lanes(lanes, layout.log_lines())
            .with_vectors()
            .with_float_rows(float_rows)
            .with_attention()
            .with_contexts(2);
        let params = EngineParams { streaming: Some(streaming), ..EngineParams::of(&layout, &store, &jobs) };
        std::fs::write(dir.path().join("engine.v"), engine_verilog(&params)).unwrap();
        std::fs::write(dir.path().join("clock.v"), COMPUTE_CLOCK_VERILOG).unwrap();
        let tb = contexts_bench(&lone_clocks(TB))
            .replace("@CHANNEL_DECLS@", RECURRENT_CHANNEL_DECLS)
            .replace("@CHANNEL_PORTS@", RECURRENT_CHANNEL_PORTS)
            .replace("@CHANNELS@", RECURRENT_CHANNELS)
            .replace("@SERVED@", SERVED)
            .replace("@STORE_PROBE@", "")
            .replace("@STORES@", "")
            .replace("@STALL@", &STALL.to_string())
            .replace("@SPACE@", "0")
            .replace("@BASE@", &map.base.to_string())
            .replace("@GL@", &GRANULE.trailing_zeros().to_string())
            .replace("@DIVIDE@", "8")
            .replace("@LINES@", &(compaction.space_bytes() as usize / 64).to_string())
            .replace("@CHANNEL_LINES@", &channels.iter().map(|c| c.len() / 64).max().unwrap().max(1).to_string())
            .replace("@AW@", &AW.to_string())
            .replace("@LW@", &LW.to_string())
            .replace("@JOBS@", &encode(&jobs).len().to_string())
            .replace("@ROWS@", &rows0.len().to_string())
            .replace("@TENSORS@", &params.tensors.max(1).to_string())
            .replace("@ROPES@", &params.ropes.max(1).to_string())
            .replace("@LAYERS@", &params.layers.max(1).to_string())
            .replace("@STORE@", &params.store_beats.to_string())
            .replace("@STORE_READ@", &logicaffeine_compile::spatial::decode_engine::rtl::ram::STORE_READ_LATENCY.to_string());
        std::fs::write(dir.path().join("tb.v"), tb).unwrap();
        let sim = Simulator::build(dir.path(), &["engine.v", "clock.v", "tb.v"]);
        let place = |at| compaction.find(at);
        let tables = [tables_at(&layout, &store, &jobs[1..], &rows0, &place), tables_at(&layout, &store, &jobs[1..], &rows1, &place)];
        let (a, b) = (&tables[0], &tables[1]);
        assert!(
            a.jobs == b.jobs && a.slots == b.slots && a.tensors == b.tensors && a.ropes == b.ropes && a.blocks == b.blocks,
            "the contexts share every table but their rows"
        );
        ContextsWorld { dir, jobs, store, rows: [rows0, rows1], snapshots: [snaps0, snaps1], tables, compaction, entries, sim, late: false }
    }

    /// The world with its channels writing each line taken 64 to 127 clocks late (`+late=1`), as channels behind a busy
    /// controller write them.
    fn late(self) -> ContextsWorld {
        ContextsWorld { late: true, ..self }
    }

    /// The step after the host's embedding on the engine for the contexts `on`, each other context given no rows: what
    /// differs from the machine — each running context's bank its step's store, each idle one's what it was given, and
    /// the flat space the memory after every running context's step — named `what`, and the cycles the engine was busy.
    fn run(&self, on: [bool; 2], what: &str) -> (Vec<String>, u64) {
        let at = self.dir.path().join(format!("contexts{}{}", on[0] as u8, on[1] as u8));
        std::fs::create_dir_all(&at).unwrap();
        let put = |name: &str, s: String| std::fs::write(at.join(name), s).unwrap();
        let t = &self.tables[0];
        put("jobs.hex", hex_lines(t.jobs.iter().map(|x| format!("{x:016x}"))));
        put("slots.hex", hex_lines(t.slots.iter().map(|x| format!("{x:016x}"))));
        put("tens.hex", hex_lines(t.tensors.iter().map(|x| format!("{x:032x}"))));
        put("ropes.hex", hex_lines(t.ropes.iter().map(|x| format!("{x:016x}"))));
        put("blocks.hex", hex_lines(t.blocks.iter().map(|f| f.iter().rev().map(|x| format!("{x:08x}")).collect())));
        let last = self.jobs.len();
        let beats = |s: &Store| hex_lines(self.store.image(s).iter().map(|beat| beat.iter().rev().map(|x| format!("{x:08x}")).collect()));
        for c in 0..2 {
            put(&format!("rows{c}.hex"), hex_lines(self.tables[c].rows.iter().map(|x| format!("{x:032x}"))));
            put(&format!("store{c}.hex"), beats(&self.snapshots[c][1].0));
        }
        // The flat space before the first running context's step and after the last's: the contexts' sequences apart, so
        // context 1's step, traced after context 0's, reads and writes what it would beside it.
        let first = if on[0] { 0 } else { 1 };
        let final_ = if on[1] { 1 } else { 0 };
        put("mem.hex", hex_lines(self.compaction.space(&self.snapshots[first][1].1, &self.entries).chunks(64).map(hex_le)));
        let run = self.sim.run(
            &at,
            &[
                format!("+rows0={}", if on[0] { self.rows[0].len() } else { 0 }),
                format!("+rows1={}", if on[1] { self.rows[1].len() } else { 0 }),
                format!("+late={}", self.late as u8),
                format!("+jobs={}", t.jobs.len()),
                format!("+f16={}", t.state_f16 as u8),
                format!("+int8={}", t.cache_int8 as u8),
                format!("+two={}", t.two_words as u8),
                format!("+perblock={}", t.per_block as u8),
            ],
        );
        let out = String::from_utf8_lossy(&run.stdout);
        if !run.status.success() || ["FAULT", "TIMEOUT", "OUTSIDE", "WRITES THE IMAGE"].iter().any(|s| out.contains(s)) {
            return (vec![format!("{what}: {out}{}", String::from_utf8_lossy(&run.stderr))], cycles_of(&out));
        }
        let mut wrong = Vec::new();
        for c in 0..2 {
            let banked = read_hex(&at.join(format!("store{c}.out")), 64);
            let want = if on[c] { &self.snapshots[c][last].0 } else { &self.snapshots[c][1].0 };
            if let Some(d) = store_differs(&banked, &self.store, want) {
                wrong.push(format!("{what}: context {c}'s bank differs at {d}"));
            }
        }
        let mem: Vec<u8> = read_hex(&at.join("mem.out"), 64).concat();
        let want = self.compaction.space(&self.snapshots[final_][last].1, &self.entries);
        if mem != want {
            let first = mem.iter().zip(&want).position(|(x, y)| x != y).unwrap_or(0);
            wrong.push(format!("{what}: the flat space differs from byte {first} (line {})", first / 64));
        }
        wrong.extend(too_quiet(what, &out));
        (wrong, cycles_of(&out))
    }
}

/// Two contexts run their steps at once on one engine with every unit on the fast side — the matvec unit streaming
/// one context's weights while the recurrence, vector and attention sides take the other's rows — and each context's
/// bank of the store and the flat space come out as the machine leaves them running the two steps one after the other.
#[test]
fn two_contexts_run_their_steps_at_once_as_the_machine_runs_them_one_after_the_other() {
    let w = ContextsWorld::of(1, logicaffeine_compile::spatial::decode_engine::rtl::vector_side::FM_ROWS);
    assert!(matches!(w.jobs[0], Job::Embed { .. }), "the step opens with the host's embedding");
    assert_none(w.run([true, true], "both contexts").0);
}

/// Two contexts' steps on head lanes: each context alone leaves the other's bank as it was given and its own as the
/// machine's step does, and both at once take fewer of the engine's cycles than the two alone do together, the units
/// serving one context while the others serve the other.
#[test]
fn two_contexts_overlap_their_units_and_each_alone_leaves_the_others_bank_untouched() {
    let w = ContextsWorld::of(2, 2);
    let (mut wrong, alone0) = w.run([true, false], "context 0 alone");
    let (more, alone1) = w.run([false, true], "context 1 alone");
    wrong.extend(more);
    let (more, both) = w.run([true, true], "both contexts");
    wrong.extend(more);
    assert_none(wrong);
    assert!(both < alone0 + alone1, "both contexts took {both} cycles, the two alone {alone0} and {alone1}");
}

/// [`two_contexts_run_their_steps_at_once_as_the_machine_runs_them_one_after_the_other`] with the channels writing
/// each line 64 to 127 clocks after taking it: context 1 alone and both contexts at once, each context's bank and the
/// flat space as the machine leaves them. Each context's sequences take two rows of the step, so a row's convolution
/// reads the ring row its sequence's row before stored, whichever context it runs for, only once that row is on the card.
#[test]
fn two_contexts_run_their_steps_at_once_with_their_writes_landing_late_as_the_machine_runs_them_one_after_the_other() {
    let w = ContextsWorld::of(1, logicaffeine_compile::spatial::decode_engine::rtl::vector_side::FM_ROWS).late();
    let (mut wrong, _) = w.run([false, true], "context 1 alone, its writes landing late");
    wrong.extend(w.run([true, true], "both contexts, their writes landing late").0);
    assert_none(wrong);
}

/// Every recurrence job of the traced step on the hybrid engine with the recurrence unit beside its streaming unit:
/// one sequence's row filling its log and flushing it, the other's rows logging its fifteenth token, its sixteenth —
/// filling its log and flushing it — and its seventeenth on the memory the flush left, the fast side's port dealing a
/// head's rows over every channel; the store and the flat space after each job the machine's, over an f16 state at
/// eight lines a granule with the engine's clock the fewest fast cycles a hybrid engine's may be, and over an f32 state
/// at sixteen lines a granule and a tenth.
#[test]
fn every_recurrence_job_runs_on_the_fast_side_as_on_the_machine() {
    let f32_state = EngineNumerics { state: StateFormat::F32, ..EngineNumerics::fastest() };
    let mut wrong = Vec::new();
    for (what, n, divide, granule) in [("f16", EngineNumerics::fastest(), ENGINE_MIN_DIVIDE, 8), ("f32", f32_state, 10, 16)] {
        let h = HybridWorld::recurrent(n, &streamed_mix, 1, divide, granule, &|v| v);
        let picked: Vec<usize> = (0..h.world.jobs.len()).filter(|&j| delta(&h.world.jobs[j])).collect();
        assert!(!picked.is_empty(), "the step's recurrence jobs");
        for j in picked {
            wrong.extend(h.run(j, j + 1, None, &format!("{what} state, divide {divide}: job {j} {:?}", h.world.jobs[j])));
        }
    }
    assert_none(wrong);
}

/// [`every_recurrence_job_runs_on_the_fast_side_as_on_the_machine`] with the recurrence unit taking a binary16 line a
/// clock (`logos_gdn_read_w`, `logos_gdn_flush_w`): every recurrence job of the traced step, the store and the flat
/// space after each the machine's, over an f16 state at eight lines a granule and an f32 state (two lines a beat) at
/// sixteen.
#[test]
fn every_recurrence_job_runs_on_the_fast_side_at_a_line_a_clock_as_on_the_machine() {
    let f32_state = EngineNumerics { state: StateFormat::F32, ..EngineNumerics::fastest() };
    let mut wrong = Vec::new();
    for (what, n, divide, granule) in [("f16", EngineNumerics::fastest(), ENGINE_MIN_DIVIDE, 8), ("f32", f32_state, 10, 16)] {
        let h = HybridWorld::recurrent_at(n, &streamed_mix, 1, divide, granule, 2, &|v| v);
        let picked: Vec<usize> = (0..h.world.jobs.len()).filter(|&j| delta(&h.world.jobs[j])).collect();
        assert!(!picked.is_empty(), "the step's recurrence jobs");
        for j in picked {
            wrong.extend(h.run(j, j + 1, None, &format!("{what} state at a line a clock, divide {divide}: job {j} {:?}", h.world.jobs[j])));
        }
    }
    assert_none(wrong);
}

/// The whole step with the recurrence unit taking a binary16 line a clock, the store and the flat space at its end the
/// machine's.
#[test]
fn the_whole_step_runs_with_its_recurrence_at_a_line_a_clock_as_on_the_machine() {
    let h = HybridWorld::recurrent_at(EngineNumerics::fastest(), &streamed_mix, 2, 8, 8, 2, &|v| v);
    assert_none(h.run(1, h.world.jobs.len(), None, "the step at a line a clock"));
}

/// [`every_recurrence_job_runs_on_head_lanes_as_on_the_machine`] with each lane's flushing head split over two flush
/// units by rows, and over four: the store and the flat space after each recurrence job the machine's, over an f16
/// state on four lanes and an f32 state on two.
#[test]
fn every_recurrence_job_runs_on_head_lanes_over_split_flushes_as_on_the_machine() {
    let f32_state = EngineNumerics { state: StateFormat::F32, ..EngineNumerics::fastest() };
    let mut wrong = Vec::new();
    for (what, n, divide, granule, lanes, flushes) in
        [("f16", EngineNumerics::fastest(), ENGINE_MIN_DIVIDE, 8, 4, 2), ("f32", f32_state, 10, 16, 2, 4)]
    {
        let sides = Sides { granule: Some(granule), width: 2, lanes, flushes, ..Sides::STREAMING };
        let h = HybridWorld::assembled(grouped(), n, &streamed_mix, [&TO_A_FLUSH, &TO_A_FLUSH[..14]], 1, divide, sides, &|v| v);
        let picked: Vec<usize> = (0..h.world.jobs.len()).filter(|&j| delta(&h.world.jobs[j])).collect();
        assert!(!picked.is_empty(), "the step's recurrence jobs");
        for j in picked {
            wrong.extend(h.run(j, j + 1, None, &format!("{what} state on {lanes} lanes over {flushes} flushes: job {j} {:?}", h.world.jobs[j])));
        }
    }
    assert_none(wrong);
}

/// Every recurrence job of the traced step on `lanes` head lanes at knob point `n`, the engine's clock divided `divide`
/// times, the flat space in granules of `granule` lines: the store and the flat space after each job the machine's.
fn every_recurrence_job_on_head_lanes(what: &str, n: EngineNumerics, divide: u32, granule: u64, lanes: usize) {
    let h = HybridWorld::recurrent_on_lanes(n, &streamed_mix, 1, divide, granule, 2, lanes, &|v| v);
    let picked: Vec<usize> = (0..h.world.jobs.len()).filter(|&j| delta(&h.world.jobs[j])).collect();
    assert!(!picked.is_empty(), "the step's recurrence jobs");
    let mut wrong = Vec::new();
    for j in picked {
        wrong.extend(h.run(j, j + 1, None, &format!("{what} state on {lanes} lanes, divide {divide}: job {j} {:?}", h.world.jobs[j])));
    }
    assert_none(wrong);
}

/// [`every_recurrence_job_runs_on_the_fast_side_at_a_line_a_clock_as_on_the_machine`] with the value heads on four head
/// lanes, a head each, each lane's rows through a fast port of its own, every head's rows dealt over every channel at
/// eight lines a granule so the lanes' ports and the coordinator's share each channel through the mux, over an f16
/// state.
#[test]
fn every_recurrence_job_runs_on_head_lanes_as_on_the_machine() {
    every_recurrence_job_on_head_lanes("f16", EngineNumerics::fastest(), ENGINE_MIN_DIVIDE, 8, 4);
}

/// [`every_recurrence_job_runs_on_head_lanes_as_on_the_machine`] over an f32 state at sixteen lines a granule.
#[test]
fn every_recurrence_job_runs_on_head_lanes_over_an_f32_state_as_on_the_machine() {
    every_recurrence_job_on_head_lanes("f32", EngineNumerics { state: StateFormat::F32, ..EngineNumerics::fastest() }, 10, 16, 4);
}

/// [`every_recurrence_job_runs_on_head_lanes_as_on_the_machine`] on two lanes, two heads each.
#[test]
fn every_recurrence_job_runs_on_two_head_lanes_as_on_the_machine() {
    every_recurrence_job_on_head_lanes("f16", EngineNumerics::fastest(), 8, 8, 2);
}

/// The whole step with every unit on the fast side and the recurrence's value heads on four head lanes — the vector
/// and attention sides on the shared port, the lanes on theirs, all on the channels through the mux — the store and the
/// flat space at its end the machine's; and with the channels giving lines back in bursts.
#[test]
fn the_whole_step_runs_with_every_unit_on_the_fast_side_and_its_recurrence_on_four_head_lanes_as_on_the_machine() {
    let h = HybridWorld::attended_on_lanes(EngineNumerics::fastest(), &streamed_mix, 2, 8, 8, 2, 4);
    let mut wrong = h.run(1, h.world.jobs.len(), None, "the step on four lanes");
    let h = h.bursting();
    wrong.extend(h.run(1, h.world.jobs.len(), None, "the step on four lanes in bursts"));
    assert_none(wrong);
}

/// The whole step on the hybrid engine with the recurrence unit beside its streaming unit — every vector and attention
/// job on the reference engine's units, every matvec job on the streaming unit, every lazily flushed memory's token-row
/// on the recurrence unit — the store and the flat space at its end the machine's, the engine's clock an eighth of the
/// units'.
#[test]
fn the_whole_step_runs_with_its_recurrence_on_the_fast_side_as_on_the_machine() {
    let h = HybridWorld::recurrent(EngineNumerics::fastest(), &streamed_mix, 2, 8, 8, &|v| v);
    assert_none(h.run(1, h.world.jobs.len(), None, "the step"));
}

// ── The simulation twin ──────────────────────────────────────────────────────────────────────────────

/// The text no engine and its twin share: the lines of `a` that `b` lacks, each as often as `a` holds it more often.
fn lines_beyond(a: &str, b: &str) -> Vec<String> {
    let mut left: Vec<&str> = b.lines().collect();
    a.lines()
        .filter(|l| match left.iter().position(|m| m == l) {
            Some(i) => {
                left.swap_remove(i);
                false
            }
            None => true,
        })
        .map(str::to_string)
        .collect()
}

/// The simulation twin ([`simulation_twin`]) is the engine with its core's clock gated to the user cycles that end with
/// the compute clock's rising edge or follow its falling edge, and nothing else changed; it refuses an engine whose core
/// could change at an edge the gate drops — a block of the core stepping at every edge, a memory reading at its step or
/// writing at every edge, an instance of the core clocked by another clock or enabled at every edge — and an engine with
/// no core instance to gate.
#[test]
fn the_simulation_twin_gates_only_an_engine_whose_core_acts_at_its_marks() {
    for ledger in ["qwen3.5-0.8b-q8_0.json", "qwen3.8-27b-q4_k_m.json"] {
        let engine = ledger_engine(ledger);
        let twin = simulation_twin(&engine);
        assert_eq!(lines_beyond(&engine, &twin), vec!["  logos_engine_core_ce core(.clk(fclk), .step_en(fedge), .read_en(cr),".to_string()], "{ledger}");
        let added = lines_beyond(&twin, &engine);
        assert_eq!(added.len(), 5, "{ledger}: {added:#?}");
        assert!(added[0].trim_start().starts_with("//"), "{ledger}: {added:#?}");
        assert_eq!(
            added[1..],
            [
                "  reg core_gate = 1'b0;",
                "  always @(negedge fclk) core_gate <= fedge || cr;",
                "  wire core_clk = fclk && core_gate;",
                "  logos_engine_core_ce core(.clk(core_clk), .step_en(fedge), .read_en(cr),",
            ],
            "{ledger}"
        );
    }
    let engine = ledger_engine("qwen3.5-0.8b-q8_0.json");
    for (what, from, to) in [
        ("a block of the core stepping at every edge", "always @(posedge clk) if (step_en) begin", "always @(posedge clk) begin"),
        ("a block of the core on the falling edge", "always @(posedge clk) if (step_en) begin", "always @(negedge clk) if (step_en) begin"),
        ("a memory reading at its step", "always @(posedge clk) if (cr) rd <= mem[ra];", "always @(posedge clk) if (ce) rd <= mem[ra];"),
        ("a memory writing at every edge", "    if (ce && we) begin", "    if (we) begin"),
        ("an instance clocked by another clock", "(.clk(clk), .step_en(step_en), .read_en(read_en), ", "(.clk(rst), .step_en(step_en), .read_en(read_en), "),
        ("a memory instance written at every edge", "(.clk(clk), .ce(step_en), .cr(read_en), ", "(.clk(clk), .ce(1'b1), .cr(read_en), "),
        ("an instance stepping at every edge", "(.clk(clk), .step_en(step_en), .read_en(read_en), ", "(.clk(clk), .step_en(1'b1), .read_en(read_en), "),
        ("no core instance to gate", ".step_en(fedge), .read_en(cr),", ".step_en(fedge), .read_en(fmid),"),
    ] {
        assert!(engine.contains(from), "{what}: the site {from:?}");
        let mutated = engine.replacen(from, to, 1);
        assert!(std::panic::catch_unwind(|| simulation_twin(&mutated)).is_err(), "{what} is refused");
    }
}

/// The engine's simulation twin and the engine it is made of run the whole step at the fastest point, flushing a log,
/// the same: each leaves the machine's store and memory, in the same number of cycles.
#[test]
fn the_engines_simulation_twin_runs_the_step_as_the_engine_does() {
    let twin = |v: String| simulation_twin(&v);
    let (mut wrong, mut cycles) = (Vec::new(), Vec::new());
    for (what, engine) in [("the engine", &(|v| v) as &dyn Fn(String) -> String), ("its twin", &twin)] {
        let w = World::built(EngineNumerics::fastest(), &matrices_in(GgmlType::Q8_0), &TO_A_FLUSH, &|p| p, engine);
        let (d, c) = w.run_counted(1, w.jobs.len(), &format!("the step on {what}"));
        wrong.extend(d);
        cycles.push(c);
    }
    assert_none(wrong);
    assert!(cycles[0] > 1000 && cycles[1] == cycles[0], "the engine's and its twin's cycles: {cycles:?}");
}

/// The hybrid engine's simulation twin and the hybrid engine run the whole step the same, the engine's clock an eighth
/// of the unit's: each leaves the machine's store and flat space, in the same number of cycles.
#[test]
fn the_hybrid_engines_simulation_twin_runs_the_step_as_the_engine_does() {
    let twin = |v: String| simulation_twin(&v);
    let (mut wrong, mut cycles) = (Vec::new(), Vec::new());
    for (what, engine) in [("the hybrid engine", &(|v| v) as &dyn Fn(String) -> String), ("its twin", &twin)] {
        let h = HybridWorld::built(EngineNumerics::fastest(), &streamed_mix, &[3, 17, 41, 8, 29], 2, 8, engine);
        let (d, c) = h.run_counted(1, h.world.jobs.len(), None, &format!("the step on {what}"));
        wrong.extend(d);
        cycles.push(c);
    }
    assert_none(wrong);
    assert!(cycles[0] > 1000 && cycles[1] == cycles[0], "the hybrid engine's and its twin's cycles: {cycles:?}");
}

/// Every vector job of the traced step — the norms, the gates, the convolution over each sequence's ring, the rotations,
/// the cache rows, the read-outs, the float matrices, the residual adds, each pass of a row and a head — on the vector
/// unit's side in the user clock beside the recurrence unit, through its port: the store and the flat space after each
/// job the machine's, over an f16 state at eight lines a granule with the engine's clock the fewest fast cycles a hybrid
/// engine's may be, and over an f32 state at sixteen lines a granule and a tenth.
#[test]
fn every_vector_job_runs_on_the_fast_side_as_on_the_machine() {
    let f32_state = EngineNumerics { state: StateFormat::F32, ..EngineNumerics::fastest() };
    let mut wrong = Vec::new();
    for (what, n, divide, granule) in [("f16", EngineNumerics::fastest(), ENGINE_MIN_DIVIDE, 8), ("f32", f32_state, 10, 16)] {
        let h = HybridWorld::vectored(n, &streamed_mix, 1, divide, granule, &|v| v);
        let picked: Vec<usize> = (0..h.world.jobs.len()).filter(|&j| vector(&h.world.jobs[j])).collect();
        assert!(!picked.is_empty(), "the step's vector jobs");
        for j in picked {
            wrong.extend(h.run(j, j + 1, None, &format!("{what} state, divide {divide}: job {j} {:?}", h.world.jobs[j])));
        }
    }
    assert_none(wrong);
}

/// The step's jobs whose passes stand alone ([`machine::VectorJob::grouped`]), of every pass a row and a head.
fn grouped_jobs(jobs: &[Job]) -> Vec<usize> {
    (0..jobs.len()).filter(|&j| matches!(&jobs[j], Job::Vector(v) if v.grouped())).collect()
}

/// Every vector job of the traced step on a vector unit's side that runs a job whose passes stand alone
/// ([`machine::VectorJob::grouped`]) in groups — as many passes as its unit holds rows for, up to the add's latency and
/// one, or three; and, on sixteen value heads of 32 ([`many_heads`]), sixteen, a reduction's in subgroups of the add's
/// latency and one — each pass loaded into rows of its own (a kept load into each pass's rows on the job's first group
/// alone), the micro-program issued once over the group, then each pass's stores from its own rows and scalars: the
/// store and the flat space after each job the machine's, every job the step runs, those in groups among them (its
/// gated norms, its head norms, its query and key norms), over an f16 state at eight lines a granule with the engine's
/// clock the fewest fast cycles a hybrid engine's may be, and over an f32 state at sixteen lines a granule and a tenth.
#[test]
fn every_vector_job_runs_on_the_fast_side_in_groups_of_passes_as_on_the_machine() {
    use logicaffeine_compile::spatial::decode_engine::rtl::fp32::FADD_LATENCY;
    let f32_state = EngineNumerics { state: StateFormat::F32, ..EngineNumerics::fastest() };
    let mut wrong = Vec::new();
    for (what, spec, n, divide, granule, passes) in [
        ("f16", grouped(), EngineNumerics::fastest(), ENGINE_MIN_DIVIDE, 8, FADD_LATENCY + 1),
        ("f32", grouped(), f32_state, 10, 16, 3),
        ("f16 on sixteen value heads", many_heads(), EngineNumerics::fastest(), ENGINE_MIN_DIVIDE, 8, 16),
    ] {
        let h = HybridWorld::vectored_in_groups_on(spec, n, &streamed_mix, divide, granule, passes);
        if passes > FADD_LATENCY + 1 {
            let side_rows = EngineParams::of(&h.world.layout, &h.world.store, &h.world.jobs).side_rows;
            let past = grouped_jobs(&h.world.jobs).into_iter().filter(|&j| {
                let Job::Vector(v) = &h.world.jobs[j] else { unreachable!() };
                side_rows / v.loads[0].len.div_ceil(LANES).max(1) > FADD_LATENCY + 1 && v.heads * h.world.rows.len() > FADD_LATENCY + 1
            });
            assert!(past.count() >= 2, "{what}: the step's jobs of more passes a group than the add's latency and one");
        }
        let picked: Vec<usize> = (0..h.world.jobs.len()).filter(|&j| vector(&h.world.jobs[j])).collect();
        let grouped = grouped_jobs(&h.world.jobs);
        assert!(grouped.len() >= 6, "{what}: the step's jobs whose passes stand alone: {grouped:?}");
        assert!(
            grouped.iter().any(|&j| matches!(&h.world.jobs[j], Job::Vector(v) if (0..v.loads.len()).any(|i| v.kept(i)))),
            "{what}: a job in groups with a kept load among the step's"
        );
        for j in picked {
            wrong.extend(h.run(j, j + 1, None, &format!("{what} state, {passes} passes a group: job {j} {:?}", h.world.jobs[j])));
        }
    }
    assert_none(wrong);
}

/// A group of more passes than the add's latency and one takes fewer clocks than groups of the add's latency and one
/// where more than that fit the side's rows: every job of the traced step whose passes stand alone, on a side of sixteen
/// passes a group and on one of five, the store and the flat space after it the machine's on both, in fewer cycles on
/// the first when more than five of its passes fit and at most as many otherwise — on sixteen value heads of 32
/// ([`many_heads`]), whose head jobs run more than five passes.
#[test]
fn a_group_past_the_adds_latency_runs_in_fewer_clocks_than_groups_of_the_adds_latency() {
    use logicaffeine_compile::spatial::decode_engine::rtl::fp32::FADD_LATENCY;
    let n = EngineNumerics::fastest();
    let add = FADD_LATENCY + 1;
    let five = HybridWorld::vectored_in_groups_on(many_heads(), n, &streamed_mix, ENGINE_MIN_DIVIDE, 8, add);
    let sixteen = HybridWorld::vectored_in_groups_on(many_heads(), n, &streamed_mix, ENGINE_MIN_DIVIDE, 8, 16);
    let side_rows = EngineParams::of(&sixteen.world.layout, &sixteen.world.store, &sixteen.world.jobs).side_rows;
    let (mut fewer, mut wrong) = (0, Vec::new());
    for j in grouped_jobs(&sixteen.world.jobs) {
        let Job::Vector(v) = &sixteen.world.jobs[j] else { unreachable!() };
        let fit = side_rows / v.loads[0].len.div_ceil(LANES).max(1);
        let passes = v.heads * sixteen.world.rows.len();
        let (w5, at5) = five.run_counted(j, j + 1, None, &format!("five a group: job {j}"));
        let (w16, at16) = sixteen.run_counted(j, j + 1, None, &format!("sixteen a group: job {j}"));
        wrong.extend(w5);
        wrong.extend(w16);
        eprintln!("job {j}: {at5} cycles five passes a group, {at16} sixteen");
        if fit > add && passes > add {
            fewer += 1;
            if at16 >= at5 {
                wrong.push(format!("job {j} {v:?}: {at16} cycles sixteen a group, {at5} five"));
            }
        } else if at16 > at5 {
            wrong.push(format!("job {j} {v:?}, of no more than five passes a group: {at16} cycles sixteen a group, {at5} five"));
        }
    }
    assert!(fewer >= 2, "the step's jobs of more than five passes a group: {fewer}");
    assert_none(wrong);
}

/// A side whose registers hold five passes of its deepest job runs that job's passes in groups, where a side a vector deep
/// runs them one after another: every job of the traced step whose passes stand alone, on sides of the add's latency and
/// one passes a group a vector deep and five vectors deep, the store and the flat space after it the machine's on both;
/// a job of more than one pass whose vectors are deeper than half the side in fewer cycles on the deep one, and no job
/// in more. A float matrix (a weight row a head) runs on the side's float-matrix path whatever its registers' depth.
#[test]
fn a_side_five_vectors_deep_runs_its_deepest_jobs_passes_in_groups() {
    use logicaffeine_compile::spatial::decode_engine::machine::{Order, Source};
    use logicaffeine_compile::spatial::decode_engine::rtl::fp32::FADD_LATENCY;
    let n = EngineNumerics::fastest();
    let world = |depth: usize| {
        let sides = Sides { granule: Some(8), vectors: true, passes: FADD_LATENCY + 1, depth, ..Sides::STREAMING };
        HybridWorld::assembled(grouped(), n, &streamed_mix, [&TO_A_FLUSH, &TO_A_FLUSH[..14]], 1, ENGINE_MIN_DIVIDE, sides, &|v| v)
    };
    let (shallow, deep) = (world(1), world(5));
    let side_rows = EngineParams::of(&deep.world.layout, &deep.world.store, &deep.world.jobs).side_rows;
    let (mut fewer, mut wrong) = (0, Vec::new());
    for j in grouped_jobs(&deep.world.jobs) {
        let Job::Vector(v) = &deep.world.jobs[j] else { unreachable!() };
        let rows = v.loads[0].len.div_ceil(LANES).max(1);
        let passes = v.heads * deep.world.rows.len();
        let (ws, at_shallow) = shallow.run_counted(j, j + 1, None, &format!("a vector deep: job {j}"));
        let (wd, at_deep) = deep.run_counted(j, j + 1, None, &format!("five vectors deep: job {j}"));
        wrong.extend(ws);
        wrong.extend(wd);
        eprintln!("job {j}: {at_shallow} cycles a vector deep, {at_deep} five vectors deep");
        let matrix = v.order == Order::Heads && v.loads.iter().any(|l| matches!(l.from, Source::Tensor { stride, .. } if stride == l.len));
        if 2 * rows > side_rows && passes > 1 && !matrix {
            fewer += 1;
            if at_deep >= at_shallow {
                wrong.push(format!("job {j} {v:?}: {at_deep} cycles five vectors deep, {at_shallow} a vector deep"));
            }
        } else if at_deep > at_shallow {
            wrong.push(format!("job {j} {v:?}: {at_deep} cycles five vectors deep, {at_shallow} a vector deep"));
        }
    }
    assert!(fewer >= 1, "the step's jobs of more than one pass as deep as the side: {fewer}");
    assert_none(wrong);
}

/// A group of passes takes fewer clocks than its passes one after another, and a job the side cannot group takes the
/// clocks it took: every job of the traced step whose passes stand alone, on a vector unit's side of the add's latency
/// and one passes a group and on one of a pass at a time, the store and the flat space after it the machine's on both,
/// in fewer cycles on the first when two of its passes fit the side's rows and in as many when one does (its vectors as
/// deep as the side) or it is a float matrix's (a tensor's row a head, on the side's path of its own).
#[test]
fn a_group_of_passes_runs_in_fewer_clocks_than_its_passes_one_after_another() {
    use logicaffeine_compile::spatial::decode_engine::machine::{Order, Source};
    use logicaffeine_compile::spatial::decode_engine::rtl::fp32::FADD_LATENCY;
    use logicaffeine_compile::spatial::decode_engine::vector::Op;
    let n = EngineNumerics::fastest();
    let alone = HybridWorld::vectored_in_groups(n, &streamed_mix, ENGINE_MIN_DIVIDE, 8, 1);
    let groups = HybridWorld::vectored_in_groups(n, &streamed_mix, ENGINE_MIN_DIVIDE, 8, FADD_LATENCY + 1);
    let side_rows = EngineParams::of(&groups.world.layout, &groups.world.store, &groups.world.jobs).side_rows;
    let grouped = grouped_jobs(&groups.world.jobs);
    let (mut fewer, mut same) = (0, 0);
    let mut wrong = Vec::new();
    for j in grouped {
        let Job::Vector(v) = &groups.world.jobs[j] else { unreachable!() };
        let float_matrix = v.order == Order::Heads
            && v.loads.iter().any(|l| matches!(l.from, Source::Tensor { stride, .. } if stride == l.len))
            && matches!(v.program[..], [Op::Dot { .. }]);
        let two_fit = side_rows / v.loads[0].len.div_ceil(LANES).max(1) >= 2 && v.heads * groups.world.rows.len() >= 2;
        let (w1, one) = alone.run_counted(j, j + 1, None, &format!("a pass at a time: job {j}"));
        let (wg, many) = groups.run_counted(j, j + 1, None, &format!("in groups: job {j}"));
        wrong.extend(w1);
        wrong.extend(wg);
        eprintln!("job {j}: {one} cycles a pass at a time, {many} in groups");
        if two_fit && !float_matrix {
            fewer += 1;
            if many >= one {
                wrong.push(format!("job {j} {v:?}: {many} cycles in groups, {one} a pass at a time"));
            }
        } else {
            same += 1;
            if many != one {
                wrong.push(format!("job {j} {v:?}, which the side cannot group: {many} cycles in groups, {one} a pass at a time"));
            }
        }
    }
    assert!(fewer >= 6 && same >= 1, "the step's jobs in groups {fewer}, alone {same}");
    assert_none(wrong);
}

/// The vector side stores into the memory a beat a clock: every job of the traced step that stores into the memory — the
/// convolutions' ring rows, the attention blocks' cache rows — the store and the flat space after it the machine's, its
/// store stream running no more clocks than the beats it asks, a fill a store (the beat's row asked, its value out of the
/// unit `SIMD_P_READ_LATENCY` clocks on and captured, its binary16 halves and int8 codes, its queue and its line: eleven
/// clocks and the unit's read) and an eighth of its beats for the port's pauses.
#[test]
fn the_vector_side_stores_into_the_memory_a_beat_a_clock() {
    use logicaffeine_compile::spatial::decode_engine::machine::Sink;
    use logicaffeine_compile::spatial::decode_engine::rtl::simd::SIMD_P_READ_LATENCY;
    const FILL: u64 = 11 + SIMD_P_READ_LATENCY as u64;
    let h = HybridWorld::vectored_in_groups(EngineNumerics::fastest(), &streamed_mix, ENGINE_MIN_DIVIDE, 8, 1);
    let mut wrong = Vec::new();
    let mut storing = 0;
    for j in 0..h.world.jobs.len() {
        let Job::Vector(v) = &h.world.jobs[j] else { continue };
        if !v.puts.iter().any(|p| matches!(p.to, Sink::Ring { .. } | Sink::Cache { .. })) {
            continue;
        }
        storing += 1;
        let (w, counted) = h.run_storing(j, j + 1, &format!("job {j}"));
        wrong.extend(w);
        let Some((beats, clocks, puts)) = counted else {
            wrong.push(format!("job {j}: no count of its stores"));
            continue;
        };
        eprintln!("job {j}: {beats} beats in {clocks} clocks over {puts} stores");
        if beats == 0 || puts == 0 || clocks > beats + beats / 8 + FILL * puts {
            wrong.push(format!("job {j} {v:?}: {beats} beats stored in {clocks} clocks over {puts} stores"));
        }
    }
    assert!(storing >= 3, "the step's jobs that store into the memory: {storing}");
    assert_none(wrong);
}

/// The vector unit's side runs a job deeper than its unit a chunk of rows at a time: every vector job of the traced step
/// that runs a chunk at a time ([`VectorJob::chunked`]) and is deeper than the side ([`EngineParams::side_rows`]) — the
/// grouped qwen35's convolutions through each sequence's ring, 1,024 channels in chunks of the side's rows, three of the
/// step's rows one sequence's tokens one after another — on the vector unit's side, the store and the flat space after
/// each the machine's.
#[test]
fn the_fast_side_runs_a_job_deeper_than_its_vector_unit_a_chunk_at_a_time_as_the_machine() {
    let h = HybridWorld::vectored(EngineNumerics::fastest(), &streamed_mix, 1, ENGINE_MIN_DIVIDE, 8, &|v| v);
    let p = EngineParams::of(&h.world.layout, &h.world.store, &h.world.jobs);
    let rows = |v: &machine::VectorJob| v.loads.iter().map(|l| l.len.div_ceil(LANES)).fold(1, usize::max);
    let deep: Vec<usize> =
        (0..h.world.jobs.len()).filter(|&j| matches!(&h.world.jobs[j], Job::Vector(v) if v.chunked() && rows(v) > p.side_rows)).collect();
    assert!(
        deep.iter().any(|&j| matches!(&h.world.jobs[j], Job::Vector(v) if v.ordered() && rows(v) > 2 * p.side_rows)),
        "a convolution of more than two chunks among the step's deep jobs: side {} rows",
        p.side_rows
    );
    let mut wrong = Vec::new();
    for j in deep {
        wrong.extend(h.run(j, j + 1, None, &format!("job {j} {:?}", h.world.jobs[j])));
    }
    assert_none(wrong);
}

/// [`grouped`] with an FFN 320 wide: its product job twenty rows, past the vector unit's side (as deep as the norms over
/// the model's 256 elements, sixteen rows), so the job runs a chunk of sixteen rows and then one of four.
fn wide_ffn() -> ModelSpec {
    let mut spec = grouped();
    for layer in &mut spec.layers {
        if let Ffn::Dense { dff, .. } = &mut layer.ffn {
            *dff = 320;
        }
    }
    spec
}

/// The vector unit's side runs a job whose last chunk is short — [`wide_ffn`]'s FFN product, 320 elements on a side
/// sixteen rows deep, a chunk of sixteen rows and then one of four — every load and store of the last chunk its four
/// rows alone: the store and the flat space after each such job the machine's. Its matrices are Q8_0, whose 32-element
/// blocks a 320-element row holds whole.
#[test]
fn the_fast_side_runs_a_job_whose_last_chunk_is_short_as_the_machine() {
    let sides = Sides { granule: Some(8), vectors: true, ..Sides::STREAMING };
    let h = HybridWorld::assembled(wide_ffn(), EngineNumerics::fastest(), &matrices_in(GgmlType::Q8_0), [&TO_A_FLUSH, &TO_A_FLUSH[..14]], 1, ENGINE_MIN_DIVIDE, sides, &|v| v);
    let p = EngineParams::of(&h.world.layout, &h.world.store, &h.world.jobs);
    let rows = |v: &machine::VectorJob| v.loads.iter().map(|l| l.len.div_ceil(LANES)).fold(1, usize::max);
    let short: Vec<usize> = (0..h.world.jobs.len())
        .filter(|&j| matches!(&h.world.jobs[j], Job::Vector(v) if v.chunked() && rows(v) > p.side_rows && rows(v) % p.side_rows != 0))
        .collect();
    assert!(!short.is_empty(), "a job whose last chunk is short, the side {} rows", p.side_rows);
    let mut wrong = Vec::new();
    for j in short {
        wrong.extend(h.run(j, j + 1, None, &format!("job {j} {:?}", h.world.jobs[j])));
    }
    assert_none(wrong);
}

/// The whole step on the hybrid engine with the recurrence unit and the vector unit's side beside its streaming unit —
/// every matvec job on the streaming unit, every lazily flushed memory's token-row on the recurrence unit, every vector
/// job on the vector unit's side, the attention jobs and every token's update on the reference engine's units — the
/// store and the flat space at its end the machine's, the engine's clock an eighth of the units'.
#[test]
fn the_whole_step_runs_with_its_vector_jobs_on_the_fast_side_as_on_the_machine() {
    let h = HybridWorld::vectored(EngineNumerics::fastest(), &streamed_mix, 2, 8, 8, &|v| v);
    assert_none(h.run(1, h.world.jobs.len(), None, "the step"));
}

/// [`grouped`] with twenty value heads over four key groups, 64 wide: its float matrices (each Gated-DeltaNet block's
/// gate and decay projections) twenty heads a job.
fn many_value_heads() -> ModelSpec {
    let mut spec = grouped();
    for layer in &mut spec.layers {
        if let Mixer::GatedDelta(g) = &mut layer.mixer {
            g.k_heads = 4;
            g.v_heads = 20;
            g.head_dim = 64;
        }
    }
    spec
}

/// Every vector job of the traced step on the vector unit's side over a model whose float matrices have twenty heads:
/// each float matrix's job on the side's own path — the rows' inputs held, eight heads' weight rows at a time (three
/// groups: eight, eight and four), each row's head through the tree into its element — and every other vector job,
/// the store and the flat space after each the machine's.
#[test]
fn every_vector_job_runs_on_the_fast_side_over_float_matrices_of_twenty_heads_as_on_the_machine() {
    let h = HybridWorld::assembled(many_value_heads(), EngineNumerics::fastest(), &streamed_mix, [&TO_A_FLUSH, &TO_A_FLUSH[..14]], 1, ENGINE_MIN_DIVIDE, Sides { granule: Some(8), vectors: true, ..Sides::STREAMING }, &|v| v);
    let picked: Vec<usize> = (0..h.world.jobs.len()).filter(|&j| vector(&h.world.jobs[j])).collect();
    assert!(picked.iter().any(|&j| matches!(&h.world.jobs[j], Job::Vector(v) if v.heads == 20)), "a float matrix of twenty heads");
    let mut wrong = Vec::new();
    for j in picked {
        wrong.extend(h.run(j, j + 1, None, &format!("job {j} {:?}", h.world.jobs[j])));
    }
    assert_none(wrong);
}

/// [`every_vector_job_runs_on_the_fast_side_over_float_matrices_of_twenty_heads_as_on_the_machine`] with the channels
/// giving lines back in bursts, back to back after a hold, as the shell's read FIFOs can: the last group's four heads'
/// lines come four apart, closer than a lane's add is deep, so each waits in the float-matrix path's queue until the sum
/// before it is written.
#[test]
fn every_vector_job_runs_on_the_fast_side_over_float_matrices_of_twenty_heads_with_lines_in_bursts_as_on_the_machine() {
    let h = HybridWorld::assembled(many_value_heads(), EngineNumerics::fastest(), &streamed_mix, [&TO_A_FLUSH, &TO_A_FLUSH[..14]], 1, ENGINE_MIN_DIVIDE, Sides { granule: Some(8), vectors: true, ..Sides::STREAMING }, &|v| v)
        .bursting();
    let picked: Vec<usize> = (0..h.world.jobs.len()).filter(|&j| vector(&h.world.jobs[j])).collect();
    assert!(picked.iter().any(|&j| matches!(&h.world.jobs[j], Job::Vector(v) if v.heads == 20)), "a float matrix of twenty heads");
    let mut wrong = Vec::new();
    for j in picked {
        wrong.extend(h.run(j, j + 1, None, &format!("job {j} {:?}", h.world.jobs[j])));
    }
    assert_none(wrong);
}

/// Every float matrix's job (heads first, a tensor's row a head) of the traced step over the twenty-head model, the
/// side's lanes taking a row fewer than `FM_ROWS` at once, reads each line of its weight rows once for every row of the
/// step and nothing else from the memory — heads × beats a row lines, its four rows two groups of its lanes, each line
/// given to them once a group — the channels giving lines back at random gaps or in bursts.
#[test]
fn every_float_matrix_job_reads_its_weight_rows_once_for_every_row() {
    use logicaffeine_compile::spatial::decode_engine::machine::{Order, Source};
    use logicaffeine_compile::spatial::decode_engine::rtl::vector_side::FM_ROWS;
    let mut wrong = Vec::new();
    for bursts in [false, true] {
        let sides = Sides { granule: Some(8), vectors: true, float_rows: FM_ROWS - 1, ..Sides::STREAMING };
        let h = HybridWorld::assembled(many_value_heads(), EngineNumerics::fastest(), &streamed_mix, [&TO_A_FLUSH, &TO_A_FLUSH[..14]], 1, ENGINE_MIN_DIVIDE, sides, &|v| v);
        let h = if bursts { h.bursting() } else { h };
        let groups = h.world.rows.len().div_ceil(sides.float_rows);
        assert!(groups > 1, "a step of {} rows in one group of {}", h.world.rows.len(), sides.float_rows);
        let mut matrices = 0;
        for j in 0..h.world.jobs.len() {
            let Job::Vector(v) = &h.world.jobs[j] else { continue };
            let Some(row) = v.loads.iter().find_map(|l| matches!(l.from, Source::Tensor { stride, .. } if stride == l.len).then_some(l.len)) else { continue };
            if v.order != Order::Heads || v.heads != 20 {
                continue;
            }
            matrices += 1;
            let what = format!("{} job {j}", if bursts { "in bursts" } else { "at gaps" });
            let (diffs, served) = h.run_served(j, j + 1, &what);
            wrong.extend(diffs);
            let want = (v.heads * row / 16) as u64;
            if served != want {
                wrong.push(format!("{what}: {served} lines read, {want} its weight rows hold, its rows {groups} groups of the lanes"));
            }
        }
        assert!(matrices >= 2, "the step's float matrices of twenty heads: {matrices}");
    }
    assert_none(wrong);
}

/// Every vector job of the traced step over the twenty-head model that reads the memory only for kept loads (a
/// tensor's span at stride 0 into a register nothing else writes: the norms' weights, the gates' constants) and stores
/// only into the token-row store reads each kept load's lines once — on its first pass, the vector unit's register
/// holding them for every pass after — across its rows and heads, the store after it the machine's, the channels
/// giving lines back at random gaps or in bursts.
#[test]
fn every_kept_load_is_read_once_a_job() {
    use logicaffeine_compile::spatial::decode_engine::machine::{Sink, Source};
    let mut wrong = Vec::new();
    for bursts in [false, true] {
        let sides = Sides { granule: Some(8), vectors: true, ..Sides::STREAMING };
        let h = HybridWorld::assembled(many_value_heads(), EngineNumerics::fastest(), &streamed_mix, [&TO_A_FLUSH, &TO_A_FLUSH[..14]], 1, ENGINE_MIN_DIVIDE, sides, &|v| v);
        let h = if bursts { h.bursting() } else { h };
        let rows = h.world.rows.len();
        let (mut picked, mut across_heads) = (0, 0);
        for j in 0..h.world.jobs.len() {
            let Job::Vector(v) = &h.world.jobs[j] else { continue };
            let kept: Vec<usize> = (0..v.loads.len()).filter(|&i| v.kept(i)).collect();
            let only_kept = (0..v.loads.len()).all(|i| v.kept(i) || matches!(v.loads[i].from, Source::Slot { .. }));
            if kept.is_empty() || !only_kept || !v.puts.iter().all(|p| matches!(p.to, Sink::Slot { .. })) {
                continue;
            }
            picked += 1;
            if v.heads > 1 {
                across_heads += 1;
            }
            let what = format!("{} job {j} ({} heads × {rows} rows)", if bursts { "in bursts" } else { "at gaps" }, v.heads);
            let (diffs, served) = h.run_served(j, j + 1, &what);
            wrong.extend(diffs);
            let want: u64 = kept.iter().map(|&i| v.loads[i].len.div_ceil(16) as u64).sum();
            if served != want {
                wrong.push(format!("{what}: {served} lines read, its kept loads' {want} once"));
            }
        }
        assert!(rows > 1 && picked >= 4 && across_heads >= 2, "{picked} jobs of kept loads over {rows} rows, {across_heads} across heads");
    }
    assert_none(wrong);
}

/// Every attention job of the traced step on the hybrid engine with the attention side beside its vector unit's side:
/// each token-row's query heads quantized and attended over its sequence's int8 cache in the user clock — one sequence
/// past a flush, the other's rows at three positions — the store and the flat space after each job the machine's, the
/// engine's clock the fewest fast cycles a hybrid engine's may be and a tenth.
#[test]
fn every_attention_job_runs_on_the_fast_side_as_on_the_machine() {
    let mut wrong = Vec::new();
    for (divide, granule) in [(ENGINE_MIN_DIVIDE, 8), (10, 16)] {
        let h = HybridWorld::attended(EngineNumerics::fastest(), &streamed_mix, 1, divide, granule, &|v| v);
        let picked: Vec<usize> = (0..h.world.jobs.len()).filter(|&j| attend(&h.world.jobs[j])).collect();
        assert!(!picked.is_empty(), "the step's attention jobs");
        for j in picked {
            wrong.extend(h.run(j, j + 1, None, &format!("divide {divide}: job {j} {:?}", h.world.jobs[j])));
        }
    }
    assert_none(wrong);
}

/// The whole step with every attention job on the attention side, every vector job on the vector unit's side, every
/// lazily flushed token-row on the recurrence unit and every matvec job on the streaming unit — the core only
/// sequencing — the store and the flat space at its end the machine's.
#[test]
fn the_whole_step_runs_with_its_attention_on_the_fast_side_as_on_the_machine() {
    let h = HybridWorld::attended(EngineNumerics::fastest(), &streamed_mix, 2, 8, 8, &|v| v);
    assert_none(h.run(1, h.world.jobs.len(), None, "the step"));
}

/// The whole step with every unit on the fast side and the channels giving lines back in bursts, back to back after a
/// hold, as the shell's read FIFOs can: every side's reads at the port's full rate after a stall.
#[test]
fn the_whole_step_runs_with_every_unit_on_the_fast_side_and_lines_in_bursts_as_on_the_machine() {
    let h = HybridWorld::attended(EngineNumerics::fastest(), &streamed_mix, 2, 8, 8, &|v| v).bursting();
    assert_none(h.run(1, h.world.jobs.len(), None, "the step in bursts"));
}

/// The whole step with every unit on the fast side and every f32 multiply and add there on the floating-point DSP block
/// (`hard_float_verilog`) — the block a stand-in for the device's (`hard_float_model`): one computing the engine's own
/// arithmetic, fully registered, and one computing IEEE 754's, which the wrappers correct, its add registered at its
/// output alone — the store and the flat space at its end the machine's.
#[test]
fn the_whole_step_runs_on_the_floating_point_dsp_blocks_as_on_the_machine() {
    use logicaffeine_compile::codegen_sva::fp_hard::Block;
    use logicaffeine_compile::codegen_sva::fp_hypotheses::Semantics;
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::{hard_float_model, hard_float_verilog, HardFloat};
    use logicaffeine_compile::spatial::decode_engine::rtl::fp32::{FADD_LATENCY, FMUL_LATENCY};
    let block = |semantics, full| Block { semantics, full, least: 1 };
    let mut wrong = Vec::new();
    for (what, hard) in [
        ("the engine's arithmetic", HardFloat { product: Some(block(Semantics::ENGINE, FMUL_LATENCY)), sum: block(Semantics::ENGINE, FADD_LATENCY), fp_unit: FpUnit::Soft }),
        ("IEEE 754's", HardFloat { product: Some(block(Semantics::IEEE, FMUL_LATENCY)), sum: block(Semantics::IEEE, FADD_LATENCY + 1), fp_unit: FpUnit::Soft }),
    ] {
        let engine = move |v: String| format!("{}\n{}", hard_float_verilog(&v, &hard), hard_float_model(&hard));
        let h = HybridWorld::attended(EngineNumerics::fastest(), &streamed_mix, 2, 8, 8, &engine);
        wrong.extend(h.run(1, h.world.jobs.len(), None, &format!("the step on a block of {what}")));
    }
    assert_none(wrong);
}

/// The whole step with every unit on the fast side and every f32 multiply and add there on the floating-point DSP
/// block, the multiplies the card's (`HardFloat { fp_unit: FpUnit::Hard, .. }`: its block's multiply, deciding a product
/// tiny at 24 bits, with nothing corrected) and the machine at the knob point of the card's products
/// (`EngineNumerics { fp_unit: FpUnit::Hard, .. }`) — the block a stand-in for the device's computing the card's own
/// multiply and the engine's add — the store and the flat space at its end the machine's.
#[test]
fn the_whole_step_runs_on_the_cards_products_as_the_machine_runs_it_at_their_knob_point() {
    use logicaffeine_compile::codegen_sva::fp_hard::Block;
    use logicaffeine_compile::codegen_sva::fp_hypotheses::Semantics;
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::{hard_float_model, hard_float_verilog, HardFloat};
    use logicaffeine_compile::spatial::decode_engine::rtl::fp32::FADD_LATENCY;
    let hard = HardFloat {
        product: Some(Block { semantics: FpUnit::CARD, full: 4, least: 1 }),
        sum: Block { semantics: Semantics::ENGINE, full: FADD_LATENCY, least: 1 },
        fp_unit: FpUnit::Hard,
    };
    let engine = move |v: String| format!("{}\n{}", hard_float_verilog(&v, &hard), hard_float_model(&hard));
    let n = EngineNumerics { fp_unit: FpUnit::Hard, ..EngineNumerics::fastest() };
    let h = HybridWorld::attended(n, &streamed_mix, 2, 8, 8, &engine);
    assert_none(h.run(1, h.world.jobs.len(), None, "the step on the card's products"));
}

/// The whole step with every unit on the fast side, its f32 adds on the floating-point DSP block and its multiplies
/// soft — the card's block flushes a tiny product before it rounds, which no wrapper turns into the engine's — the block
/// a stand-in for the device's computing IEEE 754's add, which the wrapper corrects: the store and the flat space at
/// its end the machine's.
#[test]
fn the_whole_step_runs_with_its_adds_on_the_floating_point_dsp_blocks_and_its_multiplies_soft_as_on_the_machine() {
    use logicaffeine_compile::codegen_sva::fp_hard::Block;
    use logicaffeine_compile::codegen_sva::fp_hypotheses::Semantics;
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::{hard_float_model, hard_float_verilog, HardFloat};
    use logicaffeine_compile::spatial::decode_engine::rtl::fp32::FADD_LATENCY;
    let hard = HardFloat { product: None, sum: Block { semantics: Semantics::IEEE, full: FADD_LATENCY + 1, least: 1 }, fp_unit: FpUnit::Soft };
    let engine = move |v: String| format!("{}\n{}", hard_float_verilog(&v, &hard), hard_float_model(&hard));
    let h = HybridWorld::attended(EngineNumerics::fastest(), &streamed_mix, 2, 8, 8, &engine);
    assert_none(h.run(1, h.world.jobs.len(), None, "the step on the block's adds"));
}

/// The simulation twin of the hybrid engine with the vector unit's side runs the whole step as the engine does: each
/// leaves the machine's store and flat space, in the same number of cycles.
#[test]
fn the_vectored_hybrid_engines_simulation_twin_runs_the_step_as_the_engine_does() {
    let twin = |v: String| simulation_twin(&v);
    let (mut wrong, mut cycles) = (Vec::new(), Vec::new());
    for (what, engine) in [("the engine", &(|v| v) as &dyn Fn(String) -> String), ("its twin", &twin)] {
        let h = HybridWorld::vectored(EngineNumerics::fastest(), &streamed_mix, 2, 8, 8, engine);
        let (d, c) = h.run_counted(1, h.world.jobs.len(), None, &format!("the step on {what}"));
        wrong.extend(d);
        cycles.push(c);
    }
    assert_none(wrong);
    assert!(cycles[0] > 1000 && cycles[1] == cycles[0], "the engine's and its twin's cycles: {cycles:?}");
}

/// The simulation twin of the hybrid engine with the recurrence unit beside its streaming unit runs the whole step as
/// the engine does: each leaves the machine's store and flat space, in the same number of cycles.
#[test]
fn the_recurrent_hybrid_engines_simulation_twin_runs_the_step_as_the_engine_does() {
    let twin = |v: String| simulation_twin(&v);
    let (mut wrong, mut cycles) = (Vec::new(), Vec::new());
    for (what, engine) in [("the engine", &(|v| v) as &dyn Fn(String) -> String), ("its twin", &twin)] {
        let h = HybridWorld::recurrent(EngineNumerics::fastest(), &streamed_mix, 2, 8, 8, engine);
        let (d, c) = h.run_counted(1, h.world.jobs.len(), None, &format!("the step on {what}"));
        wrong.extend(d);
        cycles.push(c);
    }
    assert_none(wrong);
    assert!(cycles[0] > 1000 && cycles[1] == cycles[0], "the engine's and its twin's cycles: {cycles:?}");
}

/// The crossing alone (`logos_engine_bridge`) between the engine clock the shell runs in and the user clock it is divided
/// from, as an engine clocked by the shell's clock meets the shell: the shell setting random values at every rising edge,
/// the engine side random values at every step (the user cycle that ends with a rising edge) and, for the store's word,
/// at every read (the user cycle after the falling edge). At six, eight and sixty-four user cycles an engine cycle, at
/// every step the engine sees exactly what the shell set at the edge before, and at every rising edge the shell sees
/// exactly what the engine set at its step before or, for the store's word, its read.
#[test]
fn the_crossing_meets_the_shell_as_an_engine_in_its_clock_does() {
    use logicaffeine_compile::spatial::decode_engine::ofs::COMPUTE_CLOCK_VERILOG;
    if !common::require_tool("iverilog") {
        return;
    }
    let bridge = logicaffeine_compile::spatial::decode_engine::rtl::engine::bridge_verilog();
    // The bridge's slow-side ports, each with its width and whether the shell drives it.
    let mut ports: Vec<(String, String, bool)> = Vec::new();
    let header = &bridge[bridge.find("(input sclk").unwrap()..bridge.find(");").unwrap()];
    for d in header.split(',').map(str::trim) {
        let into = d.starts_with("input");
        let name = d.split_whitespace().last().unwrap();
        if let Some(n) = name.strip_prefix("s_") {
            let width = d.find('[').map_or(String::new(), |a| d[a..=d.find(']').unwrap()].to_string());
            ports.push((n.to_string(), width, into));
        }
    }
    assert!(ports.len() > 20 && ports.iter().any(|(n, _, i)| n == "start" && *i) && ports.iter().any(|(n, _, i)| n == "host_rdata" && !*i));
    let random = |w: &str| if w.is_empty() { "$random".to_string() } else { "{16{$random}}".to_string() };
    let mut decls = String::new();
    let mut conns = vec![".sclk(clk), .fclk(fclk), .frst(!frst_n), .fedge(fedge), .fmid(fmid)".to_string()];
    let (mut shell, mut step, mut read, mut check) = (String::new(), String::new(), String::new(), String::new());
    for (n, w, into) in &ports {
        let w = if w.is_empty() { String::new() } else { format!("{w} ") };
        if *into {
            writeln!(decls, "  reg {w}s_{n} = 0;\n  wire {w}f_{n};").unwrap();
            writeln!(shell, "    s_{n} <= {};", random(&w)).unwrap();
            writeln!(check, "      if (f_{n} !== s_{n}) begin bad = bad + 1; $display(\"IN {n} at %0d\", cycle); end").unwrap();
        } else {
            writeln!(decls, "  wire {w}s_{n};\n  reg {w}f_{n} = 0;").unwrap();
            let into = if n == "host_rdata" { &mut read } else { &mut step };
            writeln!(into, "      f_{n} <= {};", random(&w)).unwrap();
            writeln!(check, "      if (s_{n} !== f_{n}) begin bad = bad + 1; $display(\"OUT {n} at %0d\", cycle); end").unwrap();
        }
        conns.push(format!(".s_{n}(s_{n}), .f_{n}(f_{n})"));
    }
    for divide in [6u32, 8, 64] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("bridge.v"), &bridge).unwrap();
        std::fs::write(dir.path().join("clock.v"), COMPUTE_CLOCK_VERILOG).unwrap();
        let tb = format!(
            r#"module tb;
  reg fclk = 0, frst_n = 0, cr = 0;
  wire clk, crst_n, fedge, fmid;
  logos_compute_clock #(.DIVIDE({divide})) cc(.clk(fclk), .rst_n(frst_n), .cclk(clk), .cclk_rst_n(crst_n), .fedge(fedge), .fmid(fmid));
  always #5 fclk = ~fclk;
{decls}  logos_engine_bridge u({conns});
  integer cycle = 0, bad = 0;
  // The shell: new values at every rising edge of its clock.
  always @(posedge clk) if (crst_n) begin
{shell}  end
  // The engine: new values at every step, the store's word at every read. At every step, before it: the engine sees
  // what the shell set at the edge before, and the shell what the engine set at its last step or read.
  always @(posedge fclk) begin
    cr <= fmid;
    if (fedge && crst_n) begin
      cycle = cycle + 1;
      if (cycle > 3) begin
{check}      end
{step}    end
    if (cr && crst_n) begin
{read}    end
  end
  initial begin
    #37 frst_n = 1;
    #({divide} * 10 * 300);
    $display("CYCLES %0d BAD %0d", cycle, bad);
    $finish;
  end
endmodule
"#,
            conns = conns.join(",\n    "),
        );
        std::fs::write(dir.path().join("tb.v"), tb).unwrap();
        let sim = Simulator::build(dir.path(), &["bridge.v", "clock.v", "tb.v"]);
        let run = sim.run(dir.path(), &[]);
        let out = String::from_utf8_lossy(&run.stdout).to_string();
        assert!(run.status.success(), "{divide}: {out}{}", String::from_utf8_lossy(&run.stderr));
        let cycles = cycles_of(&out);
        assert!(cycles > 250, "{divide}: {cycles} engine cycles:\n{out}");
        assert!(out.contains(" BAD 0"), "{divide}:\n{}", out.lines().take(40).collect::<Vec<_>>().join("\n"));
    }
}

/// A hybrid engine built for one activation word faults a matvec job its step runs at two: its streaming unit has no
/// second word.
#[test]
fn a_one_word_hybrid_engine_faults_a_two_word_matvec() {
    let h = HybridWorld::of(EngineNumerics::fastest(), &streamed_mix, &[3], 1, ENGINE_MIN_DIVIDE);
    let j = (0..h.world.jobs.len()).find(|&j| matvec(&h.world.jobs[j])).unwrap();
    let wrong = h.run(j, j + 1, Some(true), "a two-word job");
    assert!(wrong.len() == 1 && wrong[0].contains("FAULT"), "{wrong:?}");
}

/// Every matvec job of a traced step on `logos_streaming_matvec_<set>` built for the formats the step's matrices are
/// in at the step's activation words: the machine's store before the job in the store's block RAM, the weight image laid in stripes of `pairs` pairs a
/// channel in the four channels' memories, each answering a read descriptor with its lines at random gaps, and the job's
/// tile runs. The store after the job is the machine's: every output row the machine's values, every other beat and
/// lane as it was. With `greedy` the store is laid greedy and the head's job runs greedy: each row's beat of the logits
/// slot is then the machine's logits' arg-max and that logit, lanes 0 and 1, the beat's other lanes as they were.
/// Returned are the jobs whose store differs, named.
fn streaming_matvec_jobs(n: EngineNumerics, format_of: &dyn Fn(&str) -> GgmlType, pairs: u64, greedy: bool, token_pairs: bool, b_tokens: &[u32]) -> Vec<String> {
    use logicaffeine_compile::spatial::d5005::Beat as LineBeat;
    use logicaffeine_compile::spatial::decode_engine::image::{image_of, ImageLayout};
    use logicaffeine_compile::spatial::decode_engine::rtl::stream::{AW, LW};
    use logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet;
    use logicaffeine_compile::spatial::decode_engine::rtl::streaming::{streaming_verilog, TileRun};
    let dir = tempfile::tempdir().unwrap();
    let spec = grouped();
    let path = dir.path().join("tiny.gguf");
    tiny_models::write_checkpoint_as(&spec, &path, format_of);
    let mut machine = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), n, 2, spec.ctx_len).unwrap();
    let (a, b) = (machine.sequence().unwrap(), machine.sequence().unwrap());
    machine.step(&[(a, vec![17, 4])]).unwrap();
    let jobs = machine.jobs.clone();
    let layout = machine.layout.clone();
    let mut rows = Vec::new();
    let mut snapshots: Vec<Store> = Vec::new();
    machine
        .step_traced(&[(a, vec![60]), (b, b_tokens.to_vec())], &mut |t| {
            rows = t.rows.to_vec();
            snapshots.push(t.store.clone());
        })
        .unwrap();
    let store = if greedy { StoreLayout::greedy(&jobs, &layout, rows.len()) } else { StoreLayout::of(&jobs, &layout, rows.len()) };
    let line = ImageLayout { stripe: Some(pairs), ..ImageLayout::d5005(LineBeat::Data64) };
    let image = image_of(&layout, &line).unwrap();
    let channels = image.fill(&mut Gguf::open(&path).unwrap()).unwrap();
    for (ch, bytes) in channels.iter().enumerate() {
        std::fs::write(dir.path().join(format!("ch{ch}.hex")), hex_lines(bytes.chunks(64).map(hex_le))).unwrap();
    }
    let matvecs: Vec<(usize, usize, program::Slot, program::Slot)> =
        jobs.iter().enumerate().filter_map(|(j, job)| if let Job::Matvec { tensor, from, to, .. } = job { Some((j, *tensor, *from, *to)) } else { None }).collect();
    let mut set: Vec<GgmlType> = Vec::new();
    for &(_, tensor, _, _) in &matvecs {
        if !set.contains(&layout.tensors[tensor].ty) {
            set.push(layout.tensors[tensor].ty);
        }
    }
    let nb_max = matvecs.iter().map(|&(_, t, _, _)| layout.tensors[t].dims[0] as usize / layout.tensors[t].ty.block_len()).max().unwrap();
    let sa = (usize::BITS - (store.beats as usize - 1).leading_zeros()).max(1);
    let mix = ColumnSet::new("mix", &set, n.activations);
    let mix = if token_pairs { mix.with_token_pairs() } else { mix };
    let unit = streaming_verilog(&[mix], &line);
    assert!(!token_pairs || unit.contains(".o_val1_b(o_val1_b), .o_val2_b(o_val2_b),"), "a streaming matvec of token pairs takes its array's second token-rows' sums");
    std::fs::write(dir.path().join("unit.v"), unit).unwrap();
    let fmt = if set.len() > 1 { ".fmt(cfg[0][3:0]), " } else { "" };
    let two = if n.activations == Words::Two { ".two(cfg[3][0]), " } else { "" };
    let tb = format!(
        r#"module tb;
  localparam N = {pairs}, SB = {sb}, SA = {sa}, AW = {aw}, LW = {lw}, MDW = {aw} + {lw} + 1;
  reg clk = 0, rst = 1, start = 0;
  always #5 clk = ~clk;
  // The job: format, blocks a row, token-rows, word count, rows, stripes, the channels' turns, the input slot's first
  // beat and beats a row, the output slot's, the tensor, whether it is greedy; each channel's tile run, eight words.
  reg [31:0] cfg [0:12];
  reg [31:0] run [0:31];
  reg [511:0] st [0:SB-1];
  reg [511:0] st_rd;
  wire busy, st_we;
  wire [SA-1:0] st_ra, st_wa;
  wire [511:0] st_wd;
  wire [63:0] st_wm;
  integer w;
  always @(posedge clk) begin
    st_rd <= st[st_ra];
    if (st_we) for (w = 0; w < 64; w = w + 1) if (st_wm[w]) st[st_wa][8*w +: 8] <= st_wd[8*w +: 8];
  end
  wire [3:0] md_valid, md_ready, mr_valid, mr_ready;
  wire [4*MDW-1:0] md_data;
  wire [2047:0] mr_data;
  // Each channel's memory answers a read descriptor with its lines in order, a quarter of the clocks idle.
  genvar ch;
  generate
    for (ch = 0; ch < 4; ch = ch + 1) begin : mem
      reg [511:0] lines [0:{lines_max}];
      reg [AW-1:0] at = 0;
      reg [LW-1:0] left = 0;
      reg gap = 0;
      assign md_ready[ch] = left == 0;
      assign mr_valid[ch] = left != 0 && !gap;
      assign mr_data[512*ch +: 512] = lines[at];
      always @(posedge clk) begin
        gap <= ($random & 3) == 0;
        if (md_valid[ch] && md_ready[ch]) begin at <= md_data[MDW*ch + LW +: AW]; left <= md_data[MDW*ch +: LW]; end
        else if (mr_valid[ch] && mr_ready[ch]) begin at <= at + 1; left <= left - 1; end
      end
    end
  endgenerate
  initial begin
    $readmemh("../ch0.hex", mem[0].lines);
    $readmemh("../ch1.hex", mem[1].lines);
    $readmemh("../ch2.hex", mem[2].lines);
    $readmemh("../ch3.hex", mem[3].lines);
  end
  logos_streaming_matvec_mix #(.N(N), .T_MAX(4), .NB_MAX({nb_max}), .SA(SA)) u(.clk(clk), .rst(rst), .start(start), .greedy(cfg[12][0]), {fmt}
    .blocks(cfg[1][15:0]), .tokens(cfg[2][7:0]), {two}.rows(cfg[4]), .stripes(cfg[5][15:0]), .turns(cfg[6][7:0]),
    .from_base(cfg[7][SA-1:0]), .from_beats(cfg[8][15:0]), .to_base(cfg[9][SA-1:0]), .to_beats(cfg[10][15:0]), .busy(busy),
    .st_ra(st_ra), .st_rd(st_rd), .st_we(st_we), .st_wa(st_wa), .st_wd(st_wd), .st_wm(st_wm),
    .tr_count({{run[24], run[16], run[8], run[0]}}), .tr_line({{run[25][AW-1:0], run[17][AW-1:0], run[9][AW-1:0], run[1][AW-1:0]}}),
    .tr_lines({{run[26], run[18], run[10], run[2]}}), .tr_bytes({{run[27][23:0], run[19][23:0], run[11][23:0], run[3][23:0]}}),
    .tr_last_lines({{run[28], run[20], run[12], run[4]}}), .tr_last_bytes({{run[29][23:0], run[21][23:0], run[13][23:0], run[5][23:0]}}),
    .tr_row({{run[30], run[22], run[14], run[6]}}), .tr_row_step({{run[31], run[23], run[15], run[7]}}), .tensor(cfg[11][15:0]),
    .md_valid(md_valid), .md_ready(md_ready), .md_data(md_data), .mr_valid(mr_valid), .mr_ready(mr_ready), .mr_data(mr_data));
  integer cycles = 0;
  initial begin
    $readmemh("config.hex", cfg);
    $readmemh("runs.hex", run);
    $readmemh("store.hex", st);
    @(negedge clk); rst = 0;
    @(negedge clk); start = 1;
    @(negedge clk); start = 0;
    while (busy && cycles < 2000000) begin @(negedge clk); cycles = cycles + 1; end
    if (busy) $display("TIMEOUT");
    @(negedge clk);
    $writememh("store.out", st);
    $finish;
  end
endmodule
"#,
        sb = store.beats,
        aw = AW,
        lw = LW,
        lines_max = channels.iter().map(|c| c.len() / 64).max().unwrap(),
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let sim = Simulator::build(dir.path(), &["unit.v", "tb.v"]);
    let mut wrong = Vec::new();
    for &(j, tensor, from, to) in &matvecs {
        let t = &layout.tensors[tensor];
        let p = &image.tensors[tensor];
        let (in_f, out_f) = (t.dims[0] as usize, t.dims[1] as usize);
        let mut turns = [0usize; 4];
        for (k, &ch) in p.channels.iter().enumerate() {
            turns[ch as usize] = k;
        }
        let mut runs = Vec::new();
        for ch in 0..4u32 {
            let r = TileRun::of(&image, tensor, ch);
            runs.extend([r.count, r.line, r.lines, r.bytes, r.last_lines, r.last_bytes, r.row, r.row_step]);
        }
        let (from_base, from_beats) = store.slots[from.code() as usize];
        let (to_base, to_beats) = store.slots[to.code() as usize];
        let config = [
            set.iter().position(|&ty| ty == t.ty).unwrap(),
            in_f / t.ty.block_len(),
            rows.len(),
            (n.activations == Words::Two) as usize,
            out_f,
            out_f.div_ceil(8 * pairs as usize),
            turns.iter().enumerate().map(|(ch, &k)| k << (2 * ch)).sum(),
            from_base as usize,
            from_beats as usize,
            to_base as usize,
            to_beats as usize,
            tensor,
            (greedy && to == program::Slot::Logits) as usize,
        ];
        let at = dir.path().join(format!("job{j}"));
        std::fs::create_dir_all(&at).unwrap();
        std::fs::write(at.join("config.hex"), hex_lines(config.iter().map(|c| format!("{c:x}")))).unwrap();
        std::fs::write(at.join("runs.hex"), hex_lines(runs.iter().map(|c| format!("{c:x}")))).unwrap();
        let before = store.image(&snapshots[j]);
        std::fs::write(at.join("store.hex"), hex_lines(before.iter().map(|beat| beat.iter().rev().map(|x| format!("{x:08x}")).collect()))).unwrap();
        let run = sim.run(&at, &[]);
        let what = format!("job {j}, `{}` ({}, {out_f} rows of {in_f})", t.name, t.ty.name());
        let out = String::from_utf8_lossy(&run.stdout);
        if !run.status.success() || out.contains("TIMEOUT") {
            wrong.push(format!("{what}: {out}{}", String::from_utf8_lossy(&run.stderr)));
            continue;
        }
        // The store the job leaves: as it was, the output rows the machine's, or a greedy head's each row's arg-max.
        let mut want = before.clone();
        for (r, v) in snapshots[j + 1][&to].iter().enumerate() {
            if greedy && to == program::Slot::Logits {
                let best = logicaffeine_compile::spatial::decode_engine::numerics::argmax(&v[..out_f]);
                want[store.beat(to, r) as usize][..2].copy_from_slice(&[best as u32, v[best].to_bits()]);
                continue;
            }
            for (i, x) in v.iter().enumerate().take(out_f) {
                want[store.beat(to, r) as usize + i / LANES][i % LANES] = x.to_bits();
            }
        }
        let got = read_hex(&at.join("store.out"), 64);
        assert_eq!(got.len(), want.len(), "{what}: the store's beats");
        if let Some((k, (g, w))) = got.iter().zip(&want).enumerate().find(|(_, (g, w))| (0..LANES).any(|l| u32::from_le_bytes(g[4 * l..4 * l + 4].try_into().unwrap()) != w[l])) {
            let lane = (0..LANES).find(|&l| u32::from_le_bytes(g[4 * l..4 * l + 4].try_into().unwrap()) != w[l]).unwrap();
            wrong.push(format!("{what}: store beat {k} lane {lane}: {:#010x}, the machine {:#010x}", u32::from_le_bytes(g[4 * lane..4 * lane + 4].try_into().unwrap()), w[lane]));
        }
    }
    assert!(matvecs.len() > 8 && set.len() >= 2, "{} matvec jobs over {set:?}", matvecs.len());
    wrong
}

/// Every matvec job of the tiny model's traced step — four token-rows of two sequences — runs on the streaming matvec
/// built for the five formats the 27B's checkpoint holds its matrices in exactly as on the machine, its matrix streamed
/// in stripes from the image: one, three and four pairs a channel (a stripe's rows for a token-row in half a store beat,
/// over two beats starting half a beat in, and two whole beats as the card's ten a channel fill five), one activation
/// word and two.
#[test]
fn every_matvec_job_runs_on_the_streaming_matvec_as_on_the_machine() {
    let two = EngineNumerics { activations: Words::Two, ..EngineNumerics::fastest() };
    let mut wrong = streaming_matvec_jobs(EngineNumerics::fastest(), &streamed_mix, 1, false, false, &[5, 9, 33]);
    wrong.extend(streaming_matvec_jobs(two, &streamed_mix, 3, false, false, &[5, 9, 33]));
    wrong.extend(streaming_matvec_jobs(EngineNumerics::fastest(), &streamed_mix, 4, false, false, &[5, 9, 33]));
    assert_none(wrong);
}

/// Every matvec job of the tiny model's traced step runs on a streaming matvec of token pairs
/// (`ColumnSet::with_token_pairs`) exactly as on the machine: each issue of its array a token pair's, each entry of its
/// queue a pair's stripe, written token-row after token-row and folded or reduced as two entries — four token-rows and
/// three (the last pair one short), a greedy head's job leaving each token-row's arg-max, one, three and four pairs a
/// channel, one activation word and two.
#[test]
fn every_matvec_job_runs_on_a_streaming_matvec_of_token_pairs_as_on_the_machine() {
    let two = EngineNumerics { activations: Words::Two, ..EngineNumerics::fastest() };
    let mut wrong = streaming_matvec_jobs(EngineNumerics::fastest(), &streamed_mix, 1, false, true, &[5, 9, 33]);
    wrong.extend(streaming_matvec_jobs(two, &streamed_mix, 3, false, true, &[5, 9]));
    wrong.extend(streaming_matvec_jobs(EngineNumerics::fastest(), &streamed_mix, 4, true, true, &[5, 9]));
    wrong.extend(streaming_matvec_jobs(two, &streamed_mix, 1, true, true, &[5, 9, 33]));
    assert_none(wrong);
}

/// A greedy head's job on the streaming matvec leaves each token-row's arg-max of the machine's logits and that logit,
/// a beat a row in a greedy store, and every other matvec job of the step runs as on the machine: one, three and four
/// pairs a channel (a stripe's rows for a token-row in half a beat, over two beats starting half a beat in, and two
/// whole beats), one activation word and two.
#[test]
fn a_greedy_head_on_the_streaming_matvec_leaves_each_rows_arg_max_of_the_machines_logits() {
    let two = EngineNumerics { activations: Words::Two, ..EngineNumerics::fastest() };
    let mut wrong = streaming_matvec_jobs(EngineNumerics::fastest(), &streamed_mix, 1, true, false, &[5, 9, 33]);
    wrong.extend(streaming_matvec_jobs(two, &streamed_mix, 3, true, false, &[5, 9, 33]));
    wrong.extend(streaming_matvec_jobs(EngineNumerics::fastest(), &streamed_mix, 4, true, false, &[5, 9, 33]));
    assert_none(wrong);
}
