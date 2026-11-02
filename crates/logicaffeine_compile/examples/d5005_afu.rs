//! Write the D5005 accelerator's files (`spatial::decode_engine::ofs::afu_sources`) under a directory, the tree
//! `scripts/d5005/build-afu.sh` hands to the OFS PR flow, or a checkpoint's step program for the engine build.
//!
//!   cargo run -q -p logicaffeine-compile --example d5005_afu -- <out dir> [full72|data64|secded]
//!   cargo run -q -p logicaffeine-compile --example d5005_afu -- <out dir> data64|secded [--flat] [--floorplan <tcl>] [--user-clock <mhz>] [--divide <n>] [--hybrid <pairs> [--recurrence [--recurrence-width <w>] [--recurrence-lanes <n> [--recurrence-flushes <f> | --recurrence-behind <t>] [--recurrence-credit <lines>] [--delta-norm]] [--vectors [--float-rows <n>] [--vector-passes <n>] [--vector-depth <n>] [--conv-path <lanes>] [--attention [--contexts <n>]]]] [--greedy] [--octets [--octet-lanes <n>]] [--pe-pairs] [--token-pairs] [--slots <n>] [--folds <lanes>] [--hard-float <measurement> | --hard-adds <measurement>]] [--rows <n>] [--function-units <n>] --engine <header ledger>...
//!   cargo run --release -p logicaffeine-compile --example d5005_afu -- --program <gguf> <out dir>
//!   cargo run --release -p logicaffeine-compile --example d5005_afu -- --hybrid-program <pairs> <gguf> <out dir> [--sequences <n>] [--greedy] [--contexts <c>] [--octets] [--slots <n>] [--folds] [--delta-norm] [--behind <t>] [--hard-products] <header ledger>...
//!   cargo run --release -p logicaffeine-compile --example d5005_afu -- --decode [--hard-products] <gguf> <tokens to generate> <prompt token>...
//!   cargo run --release -p logicaffeine-compile --example d5005_afu -- --trace-store <gguf> <out dir> <prompt token>...
//!   cargo run -q -p logicaffeine-compile --example d5005_afu -- --streaming-matvec <header ledger> <out .v>
//!
//! The line the memory channels carry: `full72` the 576 bits with the ECC lanes as data (the default), `data64` the
//! 512 data bits, `secded` the 512 data bits with each word's SECDED(72,64) check byte in the ECC lanes. With
//! `--engine` the build carries the decode engine for every checkpoint the header ledgers describe at the fastest
//! point, one sequence a step over a context of at most 4096 positions — `<n>` with `--rows`, a token-row each, its
//! vector unit's functions on `<n>` function units with `--function-units` (`EngineParams::function_units`) —
//! (`EngineParams::union`), as `AfuConfig::engine` builds
//! it, its compute clock the user clock divided by `ofs::ENGINE_COMPUTE_DIVIDE` or `<n>`; with `--hybrid` the engine is
//! a hybrid one, its matvec jobs on a streaming unit in the user clock for every format the checkpoints stream
//! (`ColumnSet::hybrid`) with `<pairs>` row pairs a channel, and with `--recurrence` its lazily flushed memories'
//! token-rows on the recurrence unit beside it (`Streaming::with_recurrence`; `--recurrence-width 2` its read-out and flush
//! a binary16 line a clock, `Streaming::with_recurrence_width`; `--recurrence-lanes <n>` a row's value heads on `<n>` head
//! lanes, each on a fast port of its own, each lane's copy of the log as long as the checkpoints' longest,
//! `Streaming::with_recurrence_lanes`; `--recurrence-flushes <f>` each lane's flushing head split over `<f>` flush units
//! by rows, `Streaming::with_recurrence_flushes`; `--recurrence-behind <t>` its memories flushed a window of `<t>` tokens
//! behind, each lane replaying a share of every head's rows a token, `StateFlush::Trailing`, and so is a program written
//! with `--behind <t>`; `--recurrence-credit <lines>` the lanes and their coordinator asking up to `<lines>` lines ahead
//! of those they have taken, `Streaming::with_recurrence_credit`), and with `--vectors` its vector jobs on the
//! vector unit's side beside that, on the recurrence unit's port (`Streaming::with_vectors`; `--float-rows <n>` its float
//! matrices' inputs held `<n>` token-rows at a time, `Streaming::with_float_rows`; `--vector-passes <n>` a job whose
//! passes stand alone run `<n>` passes at a time, `Streaming::with_vector_passes`; `--vector-depth <n>` its registers
//! `<n>` passes of its deepest job deep, `Streaming::with_vector_depth`; `--conv-path <lanes>` each
//! convolution job's rows on a path of their own through `<lanes>` conv lanes, `Streaming::with_conv_path`), and with
//! `--attention` its
//! attention jobs' token-rows on the attention side beside that (`Streaming::with_attention`), and with `--contexts <n>`
//! `<n>` sequencer contexts sharing the units, each its own core, token-rows and bank of the store, the matvec unit serving
//! one while the row units serve another (`Streaming::with_contexts`); with `--greedy` its store is
//! laid greedy (`StoreLayout::greedy`), each row's logits kept as their arg-max, so it runs greedy programs of many rows;
//! with `--octets` its streaming unit takes Q8_0 matrices eight blocks of a row an issue (`ColumnSet::with_octets`), their
//! image laid in octets, and so does a program written with `--octets`, and with `--octet-lanes <n>` an octet's blocks
//! leave for `<n>` lanes a row a clock (`ColumnSet::with_octet_lanes`); with `--pe-pairs` its array's cores in pairs on
//! processing elements in uClk_usr, its user side in uClk_usrDiv2 (`ColumnSet::with_pe_pairs`); with `--token-pairs` each
//! issue of its array's held beat runs against two token-rows a clock (`ColumnSet::with_token_pairs`, two or more rows),
//! with `--pe-pairs` too on one set of each core's processing elements in uClk_usr;
//! with `--slots <n>` every block of its matrices
//! lies in a slot of a multiple of `<n>` bytes, its pad zeros (`ColumnSet::with_slots`, `ImageLayout::slot`), its feeds
//! gathering `<n>` bytes at a time, and so does a program written with `--slots <n>`; with `--folds <lanes>` its step's
//! residual adds and gated products are folded into its matrix jobs (`machine::fold`), its streaming unit folding each
//! sum into its output slot over `<lanes>` fold lanes (`Streaming::with_folds`), and a program written with `--folds`
//! folds them too; with `--delta-norm` (head lanes only) each Gated-DeltaNet block's gated norm is folded into its
//! recurrence job (`machine::fold_delta_norms`) and made on the head lanes as the read-outs are (`Streaming::with_delta_norm`),
//! and a program written with `--delta-norm` folds them too; with `--hard-float <measurement>`
//! its fast side's f32 multiplies and adds are the card's floating-point DSP blocks as the oracle's measurement on the
//! card names them (`fp_hard::Measurement::read` against the discriminator ledger its receipt in
//! `benchmarks/silicon/d5005/fp_hard_receipts.json` keeps, `Streaming::with_hard_float`), or the build is refused by
//! name — a measurement no receipt records among them — its multiplies the block's own (`FpUnit::Hard`, the card's
//! products: its programs are written with `--hard-products`); with `--hard-adds <measurement>` only its adds, the
//! multiplies soft. With `--flat` the tree is compiled into the FIM itself (`AfuConfig::flat`): its sources.txt also
//! names the FIM's static exercisers, which a FIM build with a PIM-wrapped AFU otherwise leaves out.
//! `--program` writes `ofs::write_engine_program`'s program
//! for one checkpoint at the same point: the memory flat on the channels, the engine's tables, the embeddings;
//! `--hybrid-program` `ofs::write_hybrid_program`'s for the hybrid build of the header ledgers given, its matrices in
//! stripes of `<pairs>` pairs at each channel's start and the rest of the memory compacted after them, with a region
//! for each of `<n>` sequences given `--sequences`, its head greedy given `--greedy` (`ofs::write_greedy_program`), dealt
//! over `<c>` sequencer contexts given `--contexts <c>` (`ofs::write_contexts_program`), at the card's products given
//! `--hard-products` (the knob point `fp_unit: FpUnit::Hard`, for a `--hard-float` build).
//! `--decode` prints the tokens the machine decodes greedily from a prompt at that
//! point (at the card's products given `--hard-products`), in the step program's record, each step's time on stderr: what `sw/logos_step.c` must decode on the card.
//! `--trace-store` runs the prompt on the machine and, at its last position, writes the token-row store as the card's
//! engine holds it before each mixer job (a Gated-DeltaNet or attention block's) and after the step's last job
//! (`store-<job>.bin`, each element's f32 bits), and `stops.txt`: the store's beats, then per stop its job, the job word
//! the card's engine resumes at there (the program's words open past the host's embedding), and each slot written by
//! then as its code, first beat and elements — what a card run of that position stopped at the same words must hold;
//! and `jobs.txt`, every job of the step with the word it opens at (`ofs::job_listing`, as every program carries it).
//! `--job-listing` prints a program's job listing read back from its job table alone (`ofs::job_listing_of`), for a
//! program written before programs carried theirs: a card profile's list (LOGOS_PROFILE).
//! `--streaming-matvec` writes the streaming engine's matvec (`rtl::streaming`) for the formats the checkpoint's step
//! streams at the fastest point's activation words, on the channels' 64-byte lines, its top module
//! `logos_streaming_matvec_step`.

use std::path::{Path, PathBuf};

use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::spec::ModelSpec;
use logicaffeine_compile::spatial::d5005::Beat;
use logicaffeine_compile::spatial::decode::GgufHeader;
use logicaffeine_compile::spatial::decode_engine::image::{machine_of, ImageLayout};
use logicaffeine_compile::spatial::decode_engine::machine::{encode, fold, fold_delta_norms, lower, Job, Layout, Machine, StoreLayout};
use logicaffeine_compile::spatial::decode_engine::model::{argmax, EngineNumerics};
use logicaffeine_compile::spatial::decode_engine::numerics::FpUnit;
use logicaffeine_compile::model::quant::GgmlType;
use logicaffeine_compile::spatial::decode_engine::ofs::{self, afu_sources, job_listing, job_listing_of, write_engine_program, AfuConfig, LocalMemoryLine};
use logicaffeine_compile::spatial::decode_engine::physical::PhysicalMap;
use logicaffeine_compile::spatial::decode_engine::program;
use logicaffeine_compile::codegen_sva::fp_hard::{MeasureError, Measurement};
use logicaffeine_compile::codegen_sva::fp_oracle::Oracle;
use logicaffeine_compile::codegen_sva::fp_receipt::{kept_dir, receipt_of, Ledger};
use logicaffeine_compile::spatial::decode_engine::rtl::engine::{EngineParams, HardFloat, Streaming};
use logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet;
use logicaffeine_compile::spatial::decode_engine::rtl::streaming::streaming_verilog;
use logicaffeine_compile::spatial::serving::StateFlush;

const USAGE: &str = "usage: d5005_afu <out dir> [full72|data64|secded] [--flat] [--floorplan <tcl>] [--user-clock <mhz>] [--divide <n>] [--hybrid <pairs> [--recurrence [--recurrence-width <w>] [--recurrence-lanes <n> [--recurrence-flushes <f> | --recurrence-behind <t>] [--recurrence-credit <lines>] [--delta-norm]] [--vectors [--float-rows <n>] [--vector-passes <n>] [--vector-depth <n>] [--conv-path <lanes>] [--attention [--contexts <n>]]]] [--greedy] [--octets [--octet-lanes <n>]] [--pe-pairs] [--token-pairs] [--slots <n>] [--folds <lanes>] [--hard-float <measurement> | --hard-adds <measurement>]] [--rows <n>] [--function-units <n>] [--engine <header ledger>...] \
                     | --program <gguf> <out dir> | --hybrid-program <pairs> <gguf> <out dir> [--sequences <n>] [--greedy] [--contexts <c>] [--octets] [--slots <n>] [--folds] [--delta-norm] [--behind <t>] [--hard-products] <header ledger>... \
                     | --decode [--hard-products] <gguf> <tokens to generate> <prompt token>... | --trace-store <gguf> <out dir> <prompt token>... \
                     | --job-listing <program dir> \
                     | --streaming-matvec <header ledger> <out .v>";

/// The most positions a sequence of the engine build reaches.
const CTX: usize = 4096;

/// The engine for the checkpoint a header ledger describes, at the fastest point (its memories carried as `flush` when
/// given), `rows` sequences a step (a token-row each), its store laid greedy given `greedy`, its residual adds and gated
/// products folded into its matrix jobs given `folded` and its gated norms into its recurrence jobs given `normed`, with
/// the formats of the matrices its matvec jobs stream and the
/// lines its longest log holds.
fn engine_of(ledger: &Path, rows: usize, greedy: bool, folded: bool, normed: bool, flush: Option<StateFlush>) -> (EngineParams, Vec<GgmlType>, usize) {
    let header = GgufHeader::read_ledger(ledger).unwrap_or_else(|e| panic!("{}: {e}", ledger.display()));
    let spec = header.spec().unwrap_or_else(|e| panic!("{}: {e:?}", ledger.display()));
    let fastest = EngineNumerics::fastest_for(&spec);
    let n = EngineNumerics { flush: flush.unwrap_or(fastest.flush), ..fastest };
    let steps = program::lower(&spec, n, spec.ctx_len.min(CTX)).unwrap_or_else(|e| panic!("{}: {e}", ledger.display())).program;
    let layout = Layout::of(&spec, n, &steps, &header, rows, spec.ctx_len.min(CTX)).unwrap_or_else(|e| panic!("{}: {e:?}", ledger.display()));
    let jobs = lower(&steps, &layout, spec.rms_eps as f32);
    let jobs = if folded { fold(&jobs, &layout) } else { jobs };
    let jobs = if normed { fold_delta_norms(&jobs, &layout) } else { jobs };
    let streamed = layout.tensors.iter().filter(|t| t.streamed).map(|t| t.ty).collect();
    let store = if greedy { StoreLayout::greedy(&jobs, &layout, rows) } else { StoreLayout::of(&jobs, &layout, rows) };
    (EngineParams::of(&layout, &store, &jobs), streamed, layout.log_lines())
}

/// How a hybrid build's matrices come: Q8_0's blocks eight to one of the array's or not, and the slot every block lies
/// in; and whether its array pairs its cores on processing elements in a clock of twice theirs.
#[derive(Clone, Copy)]
struct Packing {
    octets: bool,
    slot: usize,
    lanes: usize,
    pe_pairs: bool,
    token_pairs: bool,
    /// The multiplier a program is written at: the build's products.
    fp_unit: FpUnit,
    /// How a program's memories are carried, when not the fastest point's: the build's recurrence.
    flush: Option<StateFlush>,
}

/// The most beats a convolution of the checkpoints `ledgers` describe convolves a row over: each Gated-DeltaNet block's
/// queries, keys and values, sixteen channels a beat.
fn conv_beats(ledgers: &[String]) -> usize {
    let specs: Vec<ModelSpec> = ledgers
        .iter()
        .map(|l| GgufHeader::read_ledger(Path::new(l)).unwrap_or_else(|e| panic!("{l}: {e}")).spec().unwrap_or_else(|e| panic!("{l}: {e:?}")))
        .collect();
    Streaming::conv_beats(&specs)
}

/// The column set of the hybrid build of the checkpoints `ledgers` describe, packed as `packing` says.
fn hybrid_set(ledgers: &[String], packing: Packing) -> ColumnSet {
    let formats: Vec<GgmlType> = ledgers.iter().flat_map(|l| engine_of(Path::new(l), 1, false, false, false, None).1).collect();
    let set = ColumnSet::hybrid(&formats, EngineNumerics::fastest().activations);
    let set = if packing.octets { set.with_octets() } else { set };
    let set = if packing.lanes > 1 { set.with_octet_lanes(packing.lanes) } else { set };
    let set = if packing.pe_pairs { set.with_pe_pairs() } else { set };
    let set = if packing.token_pairs { set.with_token_pairs() } else { set };
    if packing.slot > 1 { set.with_slots(packing.slot) } else { set }
}

/// The image layout a hybrid build's matrices lie in: the channels' 64-byte lines in stripes of `pairs` pairs, packed
/// as `packing` says.
fn hybrid_layout(pairs: u64, packing: Packing) -> ImageLayout {
    ImageLayout { stripe: Some(pairs), octets: packing.octets, slot: packing.slot as u64, ..ImageLayout::d5005(Beat::Data64) }
}

/// The packing the flags among `args` ask for: `--octets`, `--octet-lanes <n>`, `--slots <n>`, `--pe-pairs`, `--token-pairs`.
fn packing_of(args: &[String]) -> Packing {
    let slot = args.iter().position(|a| a == "--slots").map_or(1, |s| args.get(s + 1).and_then(|n| n.parse().ok()).expect(USAGE));
    let lanes = args.iter().position(|a| a == "--octet-lanes").map_or(1, |s| args.get(s + 1).and_then(|n| n.parse().ok()).expect(USAGE));
    let octets = args.iter().any(|a| a == "--octets");
    assert!(lanes == 1 || octets, "--octet-lanes takes an octet's blocks: {USAGE}");
    let (pe_pairs, token_pairs) = (args.iter().any(|a| a == "--pe-pairs"), args.iter().any(|a| a == "--token-pairs"));
    Packing { octets, slot, lanes, pe_pairs, token_pairs, fp_unit: FpUnit::Soft, flush: None }
}

/// The floating-point DSP blocks the card's measurement (the oracle's host program's JSON at `measurement`) names,
/// against the oracle of the discriminator ledger it was measured with, the one its receipt keeps (`fp_receipt::
/// receipt_of`, by the measurement's SHA-256): its multiply and add, or with `adds` its add alone
/// (`Measurement::read_sum`, whatever its multiply measured).
fn hard_float_of(measurement: &Path, adds: bool) -> HardFloat {
    let d5005 = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/d5005");
    let read = |p: &Path| std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    let json = read(measurement);
    let receipts: Ledger = serde_json::from_str(&read(&d5005.join("fp_hard_receipts.json"))).unwrap_or_else(|e| panic!("fp_hard_receipts.json: {e}"));
    let r = receipt_of(&receipts, &json)
        .unwrap_or_else(|| panic!("{}: no receipt records this measurement (d5005_fp_receipt records one)", measurement.display()));
    let ledger = d5005.join("fp_receipts").join(kept_dir(r)).join("discriminators.json");
    let oracle = Oracle::from_ledger(&read(&ledger)).unwrap_or_else(|e| panic!("{}: {e}", ledger.display()));
    let fail = |e: MeasureError| -> ! { panic!("{}: {e}", measurement.display()) };
    if adds {
        return HardFloat { product: None, sum: Measurement::read_sum(&oracle, &json).unwrap_or_else(|e| fail(e)), fp_unit: FpUnit::Soft };
    }
    let m = Measurement::read(&oracle, &json).unwrap_or_else(|e| fail(e));
    HardFloat { product: Some(m.product), sum: m.sum, fp_unit: FpUnit::Hard }
}

/// How a hybrid program's rows run: its head greedy or not, the sequencer contexts its sequences are dealt over, and
/// whether its step's residual adds and gated products are folded into its matrix jobs and its gated norms into its
/// recurrence jobs.
#[derive(Clone, Copy)]
struct Heads {
    greedy: bool,
    contexts: usize,
    folded: bool,
    normed: bool,
}

fn write_hybrid_program(pairs: u64, gguf: &Path, out: &Path, ledgers: &[String], sequences: usize, heads: Heads, packing: Packing) {
    let set = hybrid_set(ledgers, packing);
    let g = Gguf::open(gguf).unwrap_or_else(|e| panic!("{}: {e}", gguf.display()));
    let spec = ModelSpec::from_gguf(&g).unwrap_or_else(|e| panic!("{}: {e:?}", gguf.display()));
    let ctx = spec.ctx_len.min(CTX);
    let fastest = EngineNumerics::fastest_for(&spec);
    let n = EngineNumerics { fp_unit: packing.fp_unit, flush: packing.flush.unwrap_or(fastest.flush), ..fastest };
    let machine = Machine::new(spec, g, n, sequences, ctx).unwrap_or_else(|e| panic!("{e:?}"));
    let machine = if heads.folded { machine.folded() } else { machine };
    let machine = if heads.normed { machine.delta_normed() } else { machine };
    let (map, _, compaction) = PhysicalMap::hybrid(&machine.layout, &hybrid_layout(pairs, packing), 512).unwrap_or_else(|e| panic!("{e:?}"));
    let mut weights = Gguf::open(gguf).unwrap_or_else(|e| panic!("{}: {e}", gguf.display()));
    let p = ofs::write_contexts_program(&machine, &map, &compaction, &set, &mut weights, heads.greedy, heads.contexts, out)
        .unwrap_or_else(|e| panic!("{}: {e}", out.display()));
    println!(
        "{}: {} job words, {} tokens of {} elements, vocabulary {}, {} lines a channel, streaming {}, {} sequences over {} store beats{}",
        out.display(),
        p.job_words,
        p.tokens,
        p.hidden,
        p.vocab,
        p.channel_lines[0],
        p.streaming,
        p.sequences,
        p.store_beats,
        if p.greedy { ", greedy" } else { "" }
    );
}

fn write_program(gguf: &Path, out: &Path) {
    let g = Gguf::open(gguf).unwrap_or_else(|e| panic!("{}: {e}", gguf.display()));
    let spec = ModelSpec::from_gguf(&g).unwrap_or_else(|e| panic!("{}: {e:?}", gguf.display()));
    let ctx = spec.ctx_len.min(CTX);
    let n = EngineNumerics::fastest_for(&spec);
    let machine = Machine::new(spec, g, n, 1, ctx).unwrap_or_else(|e| panic!("{e:?}"));
    let map = PhysicalMap::flat(&machine.layout, &ImageLayout::d5005(Beat::Data64), 512).unwrap_or_else(|e| panic!("{e:?}"));
    let p = write_engine_program(&machine, &map, out).unwrap_or_else(|e| panic!("{}: {e}", out.display()));
    println!("{}: {} job words, {} tokens of {} elements, vocabulary {}, {} lines a channel", out.display(), p.job_words, p.tokens, p.hidden, p.vocab, p.channel_lines[0]);
}

fn write_streaming_matvec(ledger: &Path, out: &Path) {
    let header = GgufHeader::read_ledger(ledger).unwrap_or_else(|e| panic!("{}: {e}", ledger.display()));
    let machine = machine_of(&header).unwrap_or_else(|e| panic!("{}: {e}", ledger.display()));
    let mut formats: Vec<GgmlType> = Vec::new();
    for p in machine.tensors.iter().filter(|p| p.streamed) {
        if !formats.contains(&p.ty) {
            formats.push(p.ty);
        }
    }
    let set = ColumnSet::new("step", &formats, EngineNumerics::fastest().activations);
    let v = streaming_verilog(&[set], &ImageLayout::d5005(Beat::Data64));
    std::fs::write(out, v).unwrap_or_else(|e| panic!("{}: {e}", out.display()));
    println!("{}: the streaming matvec for {}", out.display(), formats.iter().map(|t| t.name()).collect::<Vec<_>>().join(", "));
}

fn decode(gguf: &Path, generate: usize, prompt: &[u32], fp_unit: FpUnit) {
    let g = Gguf::open(gguf).unwrap_or_else(|e| panic!("{}: {e}", gguf.display()));
    let spec = ModelSpec::from_gguf(&g).unwrap_or_else(|e| panic!("{}: {e:?}", gguf.display()));
    let ctx = spec.ctx_len.min(CTX);
    let n = EngineNumerics { fp_unit, ..EngineNumerics::fastest_for(&spec) };
    let mut machine = Machine::new(spec, g, n, 1, ctx).unwrap_or_else(|e| panic!("{e:?}"));
    let s = machine.sequence().unwrap_or_else(|e| panic!("{e:?}"));
    let mut tokens = Vec::new();
    let mut token = prompt[0];
    for pos in 0..prompt.len() + generate {
        tokens.push(token);
        let start = std::time::Instant::now();
        let logits = machine.step(&[(s, vec![token])]).unwrap_or_else(|e| panic!("{e:?}")).remove(0).remove(0);
        eprintln!("position {pos}: token {token}, {:.1} s", start.elapsed().as_secs_f64());
        token = if pos + 1 < prompt.len() { prompt[pos + 1] } else { argmax(&logits) };
    }
    let list: Vec<String> = tokens.iter().map(u32::to_string).collect();
    println!("{{\"tokens\": [{}], \"prompt\": {}}}", list.join(", "), prompt.len());
}

/// The machine's store at the prompt's last position, before each mixer job and after the last (see the module's doc).
fn trace_store(gguf: &Path, out: &Path, prompt: &[u32]) {
    use std::fmt::Write;
    let g = Gguf::open(gguf).unwrap_or_else(|e| panic!("{}: {e}", gguf.display()));
    let spec = ModelSpec::from_gguf(&g).unwrap_or_else(|e| panic!("{}: {e:?}", gguf.display()));
    let ctx = spec.ctx_len.min(CTX);
    let n = EngineNumerics::fastest_for(&spec);
    let mut machine = Machine::new(spec, g, n, 1, ctx).unwrap_or_else(|e| panic!("{e:?}"));
    let s = machine.sequence().unwrap_or_else(|e| panic!("{e:?}"));
    let store = StoreLayout::of(&machine.jobs, &machine.layout, 1);
    let jobs = machine.jobs.clone();
    assert!(matches!(jobs.first(), Some(Job::Embed { .. })), "a step opens with the host's embedding");
    let mut word = vec![0usize; jobs.len() + 1];
    for j in 1..jobs.len() {
        word[j + 1] = word[j] + encode(&jobs[j..j + 1]).len();
    }
    let stops: Vec<usize> = (0..jobs.len()).filter(|&j| matches!(jobs[j], Job::Delta { .. } | Job::Attend { .. })).chain([jobs.len()]).collect();
    std::fs::create_dir_all(out).unwrap_or_else(|e| panic!("{}: {e}", out.display()));
    std::fs::write(out.join("jobs.txt"), job_listing(&jobs[1..])).unwrap_or_else(|e| panic!("{}: {e}", out.display()));
    let (last, before) = prompt.split_last().expect(USAGE);
    for (pos, &token) in before.iter().enumerate() {
        let start = std::time::Instant::now();
        machine.step(&[(s, vec![token])]).unwrap_or_else(|e| panic!("{e:?}"));
        eprintln!("position {pos}: token {token}, {:.1} s", start.elapsed().as_secs_f64());
    }
    std::fs::create_dir_all(out).unwrap_or_else(|e| panic!("{}: {e}", out.display()));
    let mut text = format!("beats {}\n", store.beats);
    machine
        .step_traced(&[(s, vec![*last])], &mut |t| {
            if !stops.contains(&t.job) {
                return;
            }
            let bits: Vec<u8> = store.image(t.store).iter().flatten().flat_map(|x| x.to_le_bytes()).collect();
            std::fs::write(out.join(format!("store-{}.bin", t.job)), bits).expect("the store's image");
            let mut slots: Vec<String> = t
                .store
                .iter()
                .map(|(slot, rows)| format!("{}:{}:{}", slot.code(), store.beat(*slot, 0), rows.first().map_or(0, Vec::len)))
                .collect();
            slots.sort();
            writeln!(text, "stop {} word {} slots {}", t.job, word[t.job.max(1)], slots.join(" ")).unwrap();
        })
        .unwrap_or_else(|e| panic!("{e:?}"));
    std::fs::write(out.join("stops.txt"), text).unwrap_or_else(|e| panic!("{}: {e}", out.display()));
    println!("{}: {} stops of {} jobs, {} beats", out.display(), stops.len(), jobs.len(), store.beats);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--trace-store") {
        let [_, gguf, out, prompt @ ..] = args.as_slice() else { panic!("{USAGE}") };
        assert!(!prompt.is_empty(), "{USAGE}");
        let prompt: Vec<u32> = prompt.iter().map(|t| t.parse().expect(USAGE)).collect();
        return trace_store(Path::new(gguf), Path::new(out), &prompt);
    }
    if args.first().map(String::as_str) == Some("--job-listing") {
        let [_, dir] = args.as_slice() else { panic!("{USAGE}") };
        print!("{}", job_listing_of(Path::new(dir)).unwrap_or_else(|e| panic!("{dir}: {e}")));
        return;
    }
    if args.first().map(String::as_str) == Some("--program") {
        let [_, gguf, out] = args.as_slice() else { panic!("{USAGE}") };
        return write_program(Path::new(gguf), Path::new(out));
    }
    if args.first().map(String::as_str) == Some("--hybrid-program") {
        let [_, pairs, gguf, out, rest @ ..] = args.as_slice() else { panic!("{USAGE}") };
        let (sequences, rest) = match rest {
            [flag, n, rest @ ..] if flag == "--sequences" => (n.parse().expect(USAGE), rest),
            rest => (1, rest),
        };
        let (greedy, rest) = match rest {
            [flag, rest @ ..] if flag == "--greedy" => (true, rest),
            rest => (false, rest),
        };
        let (contexts, rest) = match rest {
            [flag, n, rest @ ..] if flag == "--contexts" => (n.parse().expect(USAGE), rest),
            rest => (1, rest),
        };
        let (octets, rest) = match rest {
            [flag, rest @ ..] if flag == "--octets" => (true, rest),
            rest => (false, rest),
        };
        let (slot, rest) = match rest {
            [flag, n, rest @ ..] if flag == "--slots" => (n.parse().expect(USAGE), rest),
            rest => (1, rest),
        };
        let (folded, rest) = match rest {
            [flag, rest @ ..] if flag == "--folds" => (true, rest),
            rest => (false, rest),
        };
        let (normed, rest) = match rest {
            [flag, rest @ ..] if flag == "--delta-norm" => (true, rest),
            rest => (false, rest),
        };
        let (flush, rest) = match rest {
            [flag, t, rest @ ..] if flag == "--behind" => (Some(StateFlush::Trailing { tokens: t.parse().expect(USAGE) }), rest),
            rest => (None, rest),
        };
        let (fp_unit, ledgers) = match rest {
            [flag, ledgers @ ..] if flag == "--hard-products" => (FpUnit::Hard, ledgers),
            ledgers => (FpUnit::Soft, ledgers),
        };
        assert!(!ledgers.is_empty() && sequences >= 1 && contexts >= 1, "{USAGE}");
        let heads = Heads { greedy, contexts, folded, normed };
        return write_hybrid_program(pairs.parse().expect(USAGE), Path::new(gguf), Path::new(out), ledgers, sequences, heads, Packing { octets, slot, lanes: 1, pe_pairs: false, token_pairs: false, fp_unit, flush });
    }
    if args.first().map(String::as_str) == Some("--streaming-matvec") {
        let [_, ledger, out] = args.as_slice() else { panic!("{USAGE}") };
        return write_streaming_matvec(Path::new(ledger), Path::new(out));
    }
    if args.first().map(String::as_str) == Some("--decode") {
        let (fp_unit, rest) = match &args[1..] {
            [flag, rest @ ..] if flag == "--hard-products" => (FpUnit::Hard, rest),
            rest => (FpUnit::Soft, rest),
        };
        let [gguf, generate, prompt @ ..] = rest else { panic!("{USAGE}") };
        assert!(!prompt.is_empty(), "{USAGE}");
        let prompt: Vec<u32> = prompt.iter().map(|t| t.parse().expect(USAGE)).collect();
        return decode(Path::new(gguf), generate.parse().expect(USAGE), &prompt, fp_unit);
    }
    let out = PathBuf::from(args.first().expect(USAGE));
    let line = match args.get(1).map(String::as_str) {
        None | Some("full72") | Some("--engine") | Some("--flat") | Some("--floorplan") | Some("--user-clock") | Some("--divide") | Some("--hybrid") | Some("--recurrence") | Some("--recurrence-width") | Some("--recurrence-lanes") | Some("--recurrence-flushes") | Some("--recurrence-behind") | Some("--recurrence-credit") | Some("--delta-norm") | Some("--float-rows") | Some("--vector-passes") | Some("--vector-depth") | Some("--conv-path")
        | Some("--vectors") | Some("--attention") | Some("--contexts") | Some("--greedy") | Some("--octets") | Some("--octet-lanes") | Some("--pe-pairs") | Some("--token-pairs") | Some("--slots") | Some("--folds") | Some("--hard-float") | Some("--hard-adds")
        | Some("--rows") | Some("--function-units") => {
            LocalMemoryLine::DataAndEcc576
        }
        Some("data64") => LocalMemoryLine::Data512,
        Some("secded") => LocalMemoryLine::Data512Secded,
        Some(other) => panic!("{other}: {USAGE}"),
    };
    let mut cfg = AfuConfig { line, ..AfuConfig::d5005() };
    let engine_at = args.iter().position(|a| a == "--engine").unwrap_or(args.len());
    let build_form = USAGE.split(" | --program ").next().unwrap_or(USAGE);
    let build_flags: Vec<&str> = build_form.split_whitespace().map(|w| w.trim_start_matches('[').trim_end_matches(']')).filter(|w| w.starts_with("--")).collect();
    if let Some(unknown) = args[1..engine_at].iter().find(|a| a.starts_with("--") && !build_flags.contains(&a.as_str())) {
        panic!("{unknown} is no build flag: {USAGE}");
    }
    if let Some(at) = args.iter().position(|a| a == "--engine") {
        assert!(line != LocalMemoryLine::DataAndEcc576, "an engine build's lines are data64 or secded: {USAGE}");
        let rows: usize = args[..at].iter().position(|a| a == "--rows").map_or(1, |r| args.get(r + 1).and_then(|n| n.parse().ok()).expect(USAGE));
        assert!(rows >= 1, "{USAGE}");
        let greedy = args[..at].iter().any(|a| a == "--greedy");
        assert!(!greedy || args[..at].iter().any(|a| a == "--hybrid"), "--greedy is a hybrid engine's: {USAGE}");
        assert!(
            !args[..at].iter().any(|a| a == "--pe-pairs") || args[..at].iter().any(|a| a == "--hybrid"),
            "--pe-pairs pairs a streaming unit's cores: a hybrid engine's: {USAGE}"
        );
        assert!(
            !args[..at].iter().any(|a| a == "--token-pairs") || (args[..at].iter().any(|a| a == "--hybrid") && rows >= 2),
            "--token-pairs runs a streaming unit's issues two token-rows at a time: a hybrid engine's of two or more --rows: {USAGE}"
        );
        let folds: Option<usize> = args[..at].iter().position(|a| a == "--folds").map(|n| args.get(n + 1).and_then(|n| n.parse().ok()).expect(USAGE));
        assert!(folds.is_none() || args[..at].iter().any(|a| a == "--hybrid"), "--folds folds a streaming unit's sums: a hybrid engine's: {USAGE}");
        let behind: Option<u32> = args[..at].iter().position(|a| a == "--recurrence-behind").map(|n| args.get(n + 1).and_then(|n| n.parse().ok()).expect(USAGE));
        assert!(
            behind.is_none() || (args[..at].iter().any(|a| a == "--recurrence-lanes") && !args[..at].iter().any(|a| a == "--recurrence-flushes")),
            "--recurrence-behind is the head lanes', which replay its shares on one flush unit a lane: {USAGE}"
        );
        let flush = behind.map(|tokens| StateFlush::Trailing { tokens });
        let normed = args[..at].iter().any(|a| a == "--delta-norm");
        assert!(!normed || args[..at].iter().any(|a| a == "--recurrence-lanes"), "--delta-norm makes the gated norms on the head lanes: {USAGE}");
        let built: Vec<(EngineParams, Vec<GgmlType>, usize)> =
            args[at + 1..].iter().map(|l| engine_of(Path::new(l), rows, greedy, folds.is_some(), normed, flush)).collect();
        let engines: Vec<EngineParams> = built.iter().map(|b| b.0.clone()).collect();
        let log_lines = built.iter().map(|b| b.2).max().unwrap_or(0);
        assert!(!engines.is_empty(), "{USAGE}");
        let mut engine = EngineParams::union(&engines).unwrap_or_else(|e| panic!("no one engine runs them: {e}"));
        if let Some(n) = args[..at].iter().position(|a| a == "--function-units") {
            let units: usize = args.get(n + 1).and_then(|n| n.parse().ok()).expect(USAGE);
            assert!(units >= 1 && 16 % units == 0, "--function-units is a divisor of sixteen, not {units}: {USAGE}");
            engine.function_units = units;
        }
        if let Some(h) = args[..at].iter().position(|a| a == "--hybrid") {
            let pairs: u64 = args.get(h + 1).and_then(|n| n.parse().ok()).expect(USAGE);
            let has = |flag: &str| args[..at].iter().any(|a| a == flag);
            let packing = packing_of(&args[..at]);
            let streaming = Streaming::new(hybrid_set(&args[at + 1..], packing), hybrid_layout(pairs, packing));
            assert!(!has("--vectors") || has("--recurrence"), "--vectors runs on the recurrence unit's port: {USAGE}");
            let streaming = if has("--recurrence") { streaming.with_recurrence() } else { streaming };
            let width = args[..at].iter().position(|a| a == "--recurrence-width").map(|w| args.get(w + 1).and_then(|n| n.parse().ok()).expect(USAGE));
            assert!(width.is_none() || has("--recurrence"), "--recurrence-width is the recurrence unit's: {USAGE}");
            let streaming = if let Some(w) = width { streaming.with_recurrence_width(w) } else { streaming };
            let lanes = args[..at].iter().position(|a| a == "--recurrence-lanes").map(|n| args.get(n + 1).and_then(|n| n.parse().ok()).expect(USAGE));
            assert!(lanes.is_none() || has("--recurrence"), "--recurrence-lanes is the recurrence unit's: {USAGE}");
            let streaming = if let Some(n) = lanes { streaming.with_recurrence_lanes(n, log_lines) } else { streaming };
            let flushes = args[..at].iter().position(|a| a == "--recurrence-flushes").map(|n| args.get(n + 1).and_then(|n| n.parse().ok()).expect(USAGE));
            assert!(flushes.is_none() || lanes.is_some(), "--recurrence-flushes is the head lanes': {USAGE}");
            let streaming = if let Some(n) = flushes { streaming.with_recurrence_flushes(n) } else { streaming };
            let credit = args[..at].iter().position(|a| a == "--recurrence-credit").map(|n| args.get(n + 1).and_then(|n| n.parse().ok()).expect(USAGE));
            assert!(credit.is_none() || lanes.is_some(), "--recurrence-credit is the head lanes': {USAGE}");
            let streaming = if let Some(n) = credit { streaming.with_recurrence_credit(n) } else { streaming };
            let streaming = if normed { streaming.with_delta_norm() } else { streaming };
            assert!(!has("--attention") || has("--vectors"), "--attention runs beside the vector unit's side: {USAGE}");
            let streaming = if has("--vectors") { streaming.with_vectors() } else { streaming };
            let float_rows = args[..at].iter().position(|a| a == "--float-rows").map(|n| args.get(n + 1).and_then(|n| n.parse().ok()).expect(USAGE));
            assert!(float_rows.is_none() || has("--vectors"), "--float-rows is the vector unit's side's: {USAGE}");
            let streaming = if let Some(n) = float_rows { streaming.with_float_rows(n) } else { streaming };
            let passes = args[..at].iter().position(|a| a == "--vector-passes").map(|n| args.get(n + 1).and_then(|n| n.parse().ok()).expect(USAGE));
            assert!(passes.is_none() || has("--vectors"), "--vector-passes is the vector unit's side's: {USAGE}");
            let streaming = if let Some(n) = passes { streaming.with_vector_passes(n) } else { streaming };
            let depth = args[..at].iter().position(|a| a == "--vector-depth").map(|n| args.get(n + 1).and_then(|n| n.parse().ok()).expect(USAGE));
            assert!(depth.is_none() || has("--vectors"), "--vector-depth is the vector unit's side's: {USAGE}");
            let streaming = if let Some(n) = depth { streaming.with_vector_depth(n) } else { streaming };
            let conv = args[..at].iter().position(|a| a == "--conv-path").map(|n| args.get(n + 1).and_then(|n| n.parse().ok()).expect(USAGE));
            assert!(conv.is_none() || has("--vectors"), "--conv-path is the vector unit's side's: {USAGE}");
            let streaming = if let Some(n) = conv { streaming.with_conv_path(n, conv_beats(&args[at + 1..])) } else { streaming };
            let streaming = if has("--attention") { streaming.with_attention() } else { streaming };
            let contexts = args[..at].iter().position(|a| a == "--contexts").map(|n| args.get(n + 1).and_then(|n| n.parse().ok()).expect(USAGE));
            assert!(contexts.is_none() || has("--attention"), "--contexts shares the fast side's units, every one of them, the attention side too: {USAGE}");
            let streaming = if let Some(n) = contexts { streaming.with_contexts(n) } else { streaming };
            let streaming = if let Some(lanes) = folds { streaming.with_folds(lanes) } else { streaming };
            let hard = args[..at].iter().position(|a| a == "--hard-float" || a == "--hard-adds");
            assert!(!(has("--hard-float") && has("--hard-adds")), "--hard-float or --hard-adds, not both: {USAGE}");
            engine.streaming = Some(match hard {
                Some(k) => {
                    let measurement = Path::new(args.get(k + 1).expect(USAGE));
                    let hard = hard_float_of(measurement, args[k] == "--hard-adds");
                    streaming.with_hard_float(hard).unwrap_or_else(|e| panic!("{}: {e}", measurement.display()))
                }
                None => streaming,
            });
        } else {
            assert!(
                !args[..at].iter().any(|a| a == "--recurrence" || a == "--vectors" || a == "--octets" || a == "--slots" || a == "--hard-float" || a == "--hard-adds"),
                "--recurrence, --vectors, --octets, --slots, --hard-float and --hard-adds are a hybrid engine's: {USAGE}"
            );
        }
        eprintln!("{engine:?}");
        cfg = AfuConfig::engine(engine, line);
    }
    if let Some(at) = args.iter().position(|a| a == "--divide") {
        cfg.compute_divide = args.get(at + 1).and_then(|d| d.parse().ok()).expect(USAGE);
    }
    if let Some(at) = args[..engine_at].iter().position(|a| a == "--user-clock") {
        cfg.user_clock_mhz = args.get(at + 1).and_then(|m| m.parse().ok()).expect(USAGE);
    }
    if let Some(f) = args[..engine_at].iter().position(|a| a == "--floorplan") {
        let path = args.get(f + 1).expect(USAGE);
        cfg.floorplan = Some(std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}")));
    }
    cfg.flat = args[..engine_at].iter().any(|a| a == "--flat");
    for f in afu_sources(&cfg) {
        let path = out.join(f.path);
        std::fs::create_dir_all(path.parent().expect("a file under the root")).expect("the directory");
        std::fs::write(&path, &f.text).expect("the file");
        println!("{}", path.display());
    }
}
