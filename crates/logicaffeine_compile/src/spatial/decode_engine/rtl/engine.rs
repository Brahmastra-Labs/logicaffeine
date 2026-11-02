//! The engine as Verilog: the sequencer running the machine's jobs ([`crate::spatial::decode_engine::machine`]) over
//! the token-row store and the card's memory, the units doing each job's work.
//!
//! The host writes the engine's tables ([`tables`]) through its port, an entry a cycle: the jobs' words (table 0), the
//! step's row table (1: each row's region, its position and its ring index), the store's slots (2: each slot's first
//! beat and beats a row), the tensors (3) and the rotary tables (4), each block's place in a region (5), and the store
//! (6), which it reads back a beat at a time. A step runs from the word `job_at` to the word `job_end`, a job after
//! another; the host has put each row's embedding in the store, so the engine starts after the embedding's job. The card's memory is a port of whole 64-byte lines, read
//! a line a request and written a line at a time under a byte mask. A job starts at a word, runs to its end and stops;
//! the engine is busy from a step's start until its last job is done and every write the step made is on the card.
//!
//! A vector job's passes each load the vector unit's registers a beat at a time — a token-row's beat from the store,
//! a tensor's or a rotary table's line from memory, a ring row's line as the state stores it (binary16 halves widened)
//! — run the job's instructions one after another on [`super::simd`], then store a beat at a time: into the store under
//! a lane mask, or into memory as the sink keeps it (f32, binary16 halves, int8 codes, a binary16 scale) under a byte
//! mask. The vector unit's registers and scalars carry over between passes; a pass reads only what it loads and what
//! its own instructions write, so what a pass leaves never reaches the next. A slot's lanes past the width of the
//! vector a job last wrote there are no vector's: nothing reads them, so a job neither clears nor keeps them.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use crate::model::quant::GgmlType;
use crate::model::spec::{Mixer, ModelSpec};
use crate::spatial::decode_engine::machine::{encode, BlockState, Fold, Job, Layout, LogFields, Row, StoreLayout, LANES, LINE};
use crate::spatial::decode_engine::model::{CacheFormat, StateFormat};
use crate::spatial::decode_engine::numerics::{ActScaling, FpUnit, Words};
use crate::spatial::decode_engine::vector::{Bin, Func, Op};
use crate::spatial::serving::StateFlush;
use super::ram::STORE_READ_LATENCY;

use crate::codegen_sva::fp_hard::{self, hard_verilog, hard_verilog_for, plan, plan_for, Block, HardError};
use crate::codegen_sva::fp_hypotheses::Semantics;
use crate::codegen_sva::fp_oracle::{hard_model_verilog, Depths};

use super::column::ColumnSet;
use super::fp32::{FADD_LATENCY, FMUL_LATENCY};
use super::super::image::ImageLayout;

/// The engine's tables for one step, as the host writes them.
#[derive(Clone, Debug, PartialEq)]
pub struct Tables {
    /// The jobs' words ([`crate::spatial::decode_engine::machine::encode`]).
    pub jobs: Vec<u64>,
    /// Per row: its region's first line `[31:0]`, its position `[63:32]`, its ring index `[71:64]`.
    pub rows: Vec<u128>,
    /// Per slot code: its first beat `[31:0]`, its beats a row `[63:32]`.
    pub slots: Vec<u64>,
    /// Per tensor: its first line `[31:0]`, its ggml type `[39:32]`, its row's elements `[63:40]`, its rows `[87:64]`.
    pub tensors: Vec<u128>,
    /// Per rotary table: its first line `[31:0]`, its pairs `[63:32]`.
    pub ropes: Vec<u64>,
    /// Per layer, sixteen 32-bit fields from the lowest. A Gated-DeltaNet block's: its ring's offset in bytes, its
    /// channels, its ring rows, its memory's and its log's offsets, a log entry's bytes, its key and value heads, its
    /// width, its log entry's values, decay exponents and gates (elements), the tokens of a flush (0 for a memory
    /// rewritten every pass), its read-out's scale's bits. An attention block's: its keys' and values' offsets, its key
    /// and value scales' offsets, its cached heads, its widths of key and value. The last field 0 for a Gated-DeltaNet
    /// block, 1 for an attention block.
    pub blocks: Vec<[u32; 16]>,
    /// Whether the state stores binary16, whether the cache stores int8 codes, whether an activation crosses the array
    /// in two words, whether activations are scaled per weight block.
    pub state_f16: bool,
    pub cache_int8: bool,
    pub two_words: bool,
    pub per_block: bool,
}

/// The tokens a Gated-DeltaNet memory carried as `flush` logs before the engine replays them: 1 for a memory rewritten
/// every pass. A unit that replays a whole log at once runs these; a trailing flush runs on head lanes alone
/// ([`recurrence_log`]).
pub fn logged(flush: StateFlush) -> usize {
    match flush {
        StateFlush::EveryPass => 1,
        StateFlush::Every { tokens } => tokens as usize,
        StateFlush::Trailing { .. } => panic!("a trailing flush, which only the recurrence's head lanes replay"),
    }
}

/// The part of a streaming unit's name a recurrence carrying its memories as `flush` adds: `+behind<T>` for a trailing
/// flush of `T`, nothing otherwise.
pub fn behind(flush: Option<StateFlush>) -> String {
    match flush {
        Some(StateFlush::Trailing { tokens }) => format!("+behind{tokens}"),
        _ => String::new(),
    }
}

/// The tokens a recurrence unit's log holds at its longest and the window a trailing flush replays a window behind:
/// [`logged`]'s, or a trailing flush's two windows and its window.
pub fn recurrence_log(flush: StateFlush) -> (usize, Option<usize>) {
    match flush {
        StateFlush::Trailing { tokens } => (2 * tokens as usize, Some(tokens as usize)),
        f => (logged(f), None),
    }
}

/// The tables of `jobs` over `layout`, the store laid out as `store`, for the step's `rows`.
pub fn tables(layout: &Layout, store: &StoreLayout, jobs: &[Job], rows: &[Row]) -> Tables {
    tables_at(layout, store, jobs, rows, &|at| Some(at))
}

/// [`tables`] with every address the engine reads the card's memory at — a tensor's, a rotary table's, a sequence's
/// region — placed by `place`: the hybrid engine's tables over its compacted memory
/// ([`crate::spatial::decode_engine::physical::Compaction::place`]). A tensor `place` gives no place, one the hybrid
/// engine streams and never reads by address, has address 0.
pub fn tables_at(layout: &Layout, store: &StoreLayout, jobs: &[Job], rows: &[Row], place: &dyn Fn(u64) -> Option<u64>) -> Tables {
    let placed = |at: u64| place(at).expect("a place for every region and rotary table");
    let line = |bytes: u64| -> u32 {
        assert_eq!(bytes % LINE, 0, "a place off a line");
        u32::try_from(bytes / LINE).expect("a line within 32 bits")
    };
    Tables {
        jobs: encode(jobs),
        rows: rows
            .iter()
            .map(|r| line(placed(layout.region_of(r.sequence))) as u128 | (r.position as u128) << 32 | (r.ring as u128) << 64)
            .collect(),
        slots: store.slots.iter().map(|&(base, beats)| base as u64 | (beats as u64) << 32).collect(),
        tensors: layout
            .tensors
            .iter()
            .map(|t| {
                let dims = |i: usize| t.dims.get(i).copied().unwrap_or(1) as u128;
                line(place(t.at).unwrap_or(0)) as u128 | (t.ty.id() as u128) << 32 | dims(0) << 40 | dims(1) << 64
            })
            .collect(),
        ropes: layout.rotary.iter().map(|(rope, at)| line(placed(*at)) as u64 | ((rope.dims / 2) as u64) << 32).collect(),
        blocks: layout
            .blocks
            .iter()
            .map(|b| match *b {
                BlockState::Delta { ring, channels, taps, memory, log, entry, k_heads, v_heads, width } => {
                    let f = LogFields::of(k_heads, v_heads, width);
                    let tokens = match layout.numerics.flush {
                        StateFlush::EveryPass => 0,
                        StateFlush::Trailing { tokens } => tokens,
                        flush => logged(flush) as u32,
                    };
                    [
                        ring as u32,
                        channels as u32,
                        taps.saturating_sub(1) as u32,
                        memory as u32,
                        log.unwrap_or(0) as u32,
                        entry as u32,
                        k_heads as u32,
                        v_heads as u32,
                        width as u32,
                        f.values as u32,
                        f.log_decays as u32,
                        f.gates as u32,
                        tokens,
                        ((1.0 / (width as f64).sqrt()) as f32).to_bits(),
                        0,
                        0,
                    ]
                }
                BlockState::Attention { keys, values, key_scales, value_scales, kv_heads, width, value_width } => [
                    keys as u32,
                    values as u32,
                    key_scales.unwrap_or(0) as u32,
                    value_scales.unwrap_or(0) as u32,
                    kv_heads as u32,
                    width as u32,
                    value_width as u32,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    1,
                ],
            })
            .collect(),
        state_f16: layout.numerics.state == StateFormat::F16,
        cache_int8: layout.numerics.cache != CacheFormat::F16,
        two_words: layout.numerics.activations == Words::Two,
        per_block: layout.numerics.scaling == ActScaling::PerBlock,
    }
}

/// What an engine is printed for: the most token-rows a step holds, the most beats a vector register holds and the
/// most a Hadamard or rotary operand does, the store's beats, the words of the step's jobs, the tables' entries,
/// every block format its matrices are in with the most blocks a row of that format holds, and the function units its
/// vector unit's sixteen lanes share.
#[derive(Clone, Debug, PartialEq)]
pub struct EngineParams {
    pub rows: usize,
    pub vector_rows: usize,
    /// The beats the vector unit's side holds a register to: the deepest vector job it cannot run a chunk at a time
    /// ([`super::super::machine::VectorJob::chunked`]); a deeper job runs that many rows of its vectors at a time. The
    /// reference engine's own vector unit runs every job whole, as deep as [`EngineParams::vector_rows`].
    pub side_rows: usize,
    /// The beats of the vector unit's pair copies: a far Hadamard stage's partner rows and a rotary block's rows are
    /// rows of the instruction's operand, so they are within the longest operand a Hadamard or rotary job loads.
    pub pair_rows: usize,
    pub store_beats: usize,
    pub job_words: usize,
    pub tensors: usize,
    pub layers: usize,
    pub ropes: usize,
    pub formats: Vec<(GgmlType, usize)>,
    /// The Gated-DeltaNet blocks' head width and how their memories are carried ([`logged`]), or `None` for a model of
    /// none.
    pub delta: Option<(usize, StateFlush)>,
    /// The attention blocks' query and value head widths, the most positions a sequence reaches, and whether the math
    /// runs on the array, or `None` for a model of none.
    pub attend: Option<(usize, usize, usize, bool)>,
    /// The vector unit's function units, a divisor of sixteen: a function's row takes `16 / function_units` cycles.
    pub function_units: usize,
    /// Whether a matrix job folds its sums into its output slot ([`super::super::machine::Fold`]): a step lowered with
    /// its residual adds and gated products folded ([`super::super::machine::fold`]).
    pub folds: bool,
    /// The streaming unit a hybrid engine runs its matvec jobs on, or `None` for the reference engine, which reads each
    /// job's matrix a line at a time through its memory port.
    pub streaming: Option<Streaming>,
}

/// The streaming unit of a hybrid engine ([`super::streaming::streaming_verilog`]), in the fast clock beside the
/// engine's: the column set it is built for and the image layout its matrices lie in on the channels, whose stripe is
/// the row pairs a channel's share of a stripe holds; and whether the recurrence unit and the vector unit's side stand
/// beside it.
#[derive(Clone, Debug, PartialEq)]
pub struct Streaming {
    pub set: ColumnSet,
    pub layout: ImageLayout,
    /// Whether a lazily flushed memory's token-rows run on the fast side too (`logos_gdn_side` on the channels through
    /// `logos_fast_port`, [`GdnJob`]): each row's log entry and count, fold, read-out and flush at the channels' rate in
    /// the user clock, the core handing the row over and waiting for it. The unit walks a head's memory a row of whole
    /// lines at a time, so it serves heads whose binary16 rows are whole lines, a width a multiple of 32.
    pub recurrence: bool,
    /// The sixteens of elements a clock the recurrence unit's read-out and flush take: 1 half a binary16 line
    /// (`logos_gdn_read_p`, `logos_gdn_flush_p`), 2 a whole line (`logos_gdn_read_w`, `logos_gdn_flush_w`, the flush
    /// holding a head's rows and running its dot and update passes beside each other).
    pub recurrence_width: usize,
    /// The head lanes the recurrence unit runs a row's value heads on: 1 the side alone; more a coordinator and that
    /// many lanes (`logos_gdn_lanes`, [`super::gdn::lanes_verilog`]), lane k taking value heads k, k + lanes, … through
    /// its own fast port, the lanes' ports and the shared one on the channels at once (`logos_channel_mux`), so with
    /// four lanes and a head a granule each lane reads and writes a channel of its own.
    pub recurrence_lanes: usize,
    /// The lines each lane's copy of the log holds ([`super::super::machine::Layout::log_lines`] of the layouts the
    /// build runs), a lanes build only: a row whose log would take more faults the recurrence unit, which runs none of
    /// it.
    pub log_lines: usize,
    /// The flush units each head lane splits a flushing head's rows over, a lanes build at a binary16 line a clock only:
    /// each unit holds HD / flushes of the rows through the same passes over the log, so a flushing head's passes take
    /// 1 / flushes of the clocks and every row flushes as one unit flushes it. 1 a unit a lane.
    pub recurrence_flushes: usize,
    /// The lines the recurrence's coordinator and each head lane ask ahead of those they have taken, a lanes build only
    /// ([`super::gdn::LANE_CREDIT`] unless given): behind a memory that answers `latency` clocks after an ask, a lane
    /// reads at most `credit / latency` lines a clock, so a credit past the channels' latency reads a line a clock.
    pub recurrence_credit: usize,
    /// Whether the vector jobs run on the fast side too (`logos_vector_side`, [`super::vector_side`], on the recurrence
    /// unit's port, [`VectorSideJob`]): each job's loads, micro-program and stores in the user clock on the user clock's
    /// vector unit, the core handing the job over and waiting for it.
    pub vectors: bool,
    /// Whether the attention jobs run on the fast side too (`logos_attend_side`, [`super::attend`], on the recurrence
    /// unit's port, [`AttendJob`]): each token-row's query heads quantized and attended over the int8 cache in the user
    /// clock, the core handing each row over and waiting for it; the core keeps no attention unit or quantizer.
    pub attention: bool,
    /// The card's floating-point DSP block the fast side's f32 multiply and add are built on ([`HardFloat`]), or `None`
    /// for both in logic.
    pub hard_float: Option<HardFloat>,
    /// The token-rows the vector unit's side holds the inputs of while a float matrix's weights stream past
    /// ([`super::vector_side::FM_ROWS`] unless given): 16 lanes a row, the weights read once for each group.
    pub float_rows: usize,
    /// The passes of a job whose passes stand alone ([`super::super::machine::VectorJob::grouped`]) the vector unit's
    /// side runs at once, each pass's registers rows and scalars of its own (`logos_simd_p`'s `PASSES`), so a pass's
    /// reductions and functions fill the pipeline its one chain leaves idle: 1 a pass at a time; at most the add's
    /// latency and one, the slots a round of the unit's reductions interleaves.
    pub vector_passes: usize,
    /// The passes of the deepest vector job it runs whole the vector unit's side's registers hold
    /// ([`Streaming::with_vector_depth`]): its unit that many times as deep as that job's vectors, so a job whose vectors
    /// are as deep as the side runs in groups as a shallower job's do. 1 a register a vector deep.
    pub vector_depth: usize,
    /// The sequencer contexts sharing the units: each a core of its own over its own token-rows, its rows table and its
    /// bank of the token-row store, the matvec unit serving one context while the row units (the recurrence, vector and
    /// attention sides) serve another, so one context's weights stream while another's rows run. 1 for one sequencer.
    pub contexts: usize,
    /// The fold lanes the streaming unit folds a job's sums into its output slot over
    /// ([`super::super::machine::Fold`], [`super::streaming::streaming_verilog_folding`]): a beat each `16 / folds`
    /// clocks, read from the slot, folded and written back; 0 a unit that writes each sum as it is.
    pub folds: usize,
    /// Whether a recurrence job's gated norm ([`super::super::machine::DeltaNorm`]) is made on its head lanes
    /// ([`super::gdn::delta_normed_lanes`]): each lane's read-outs normed by the job's weight and gated by the SiLU of
    /// the row's gate as they are made, so a step whose gated norms are folded into its recurrence jobs
    /// ([`super::super::machine::fold_delta_norms`]) runs on it, a vector job fewer a Gated-DeltaNet block.
    pub delta_norm: bool,
    /// The lanes a convolution job's rows run through on the vector unit's side's path of its own
    /// ([`super::vector_side::conv_path_verilog`]), 0 for none, and the most beats of its channels a tap of the kernel
    /// the path holds.
    pub conv_lanes: usize,
    pub conv_beats: usize,
}

/// The floating-point DSP block's add, and its multiply unless the multiplies stay soft, as the oracle measured them
/// ([`crate::codegen_sva::fp_hard::Block`]): a hybrid engine whose fast side builds `logos_fadd_p`, and `logos_fmul_p`
/// given a `product`, on the block ([`crate::codegen_sva::fp_hard::hard_verilog_for`]), each the soft unit's ports and
/// latency, the logic around the block correcting what it computes otherwise. The add computes the engine's own
/// arithmetic; the multiply the arithmetic of `fp_unit`: at [`FpUnit::Soft`] the engine's, so a block whose multiply
/// decides a product tiny before the engine does is refused, at [`FpUnit::Hard`] the card's ([`FpUnit::CARD`]), the
/// arithmetic the numerics of an `fp_unit: Hard` knob point multiply by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HardFloat {
    pub product: Option<Block>,
    pub sum: Block,
    pub fp_unit: FpUnit,
}

impl HardFloat {
    /// The arithmetic the product's wrapper computes: the engine's at [`FpUnit::Soft`], the card's at [`FpUnit::Hard`].
    pub fn product_target(&self) -> Semantics {
        match self.fp_unit {
            FpUnit::Soft => Semantics::ENGINE,
            FpUnit::Hard => FpUnit::CARD,
        }
    }
}

impl Streaming {
    /// The unit for `set` over `layout`, which lays its streamed matrices in stripes.
    pub fn new(set: ColumnSet, layout: ImageLayout) -> Streaming {
        assert!(layout.stripe.is_some_and(|n| n > 0), "a streaming unit's matrices lie in stripes");
        assert!(
            layout.octets == set.octets && layout.slot == set.slot as u64,
            "a streaming unit's set and its image lay blocks alike: octets {} and {}, slots {} and {}",
            set.octets,
            layout.octets,
            set.slot,
            layout.slot
        );
        Streaming {
            set,
            layout,
            recurrence: false,
            recurrence_width: 1,
            recurrence_lanes: 1,
            log_lines: 0,
            recurrence_flushes: 1,
            recurrence_credit: super::gdn::LANE_CREDIT,
            vectors: false,
            attention: false,
            hard_float: None,
            float_rows: super::vector_side::FM_ROWS,
            vector_passes: 1,
            vector_depth: 1,
            contexts: 1,
            folds: 0,
            delta_norm: false,
            conv_lanes: 0,
            conv_beats: 0,
        }
    }

    /// The unit with each convolution job (the machine's conv job, four taps, its ring rows whole lines) run on the
    /// vector unit's side's path of its own: the kernel read once a job, each row's ring rows and x streamed through
    /// `lanes` conv lanes (1, 2, 4, 8 or 16 channels a clock), its output and ring row written as they come; `beats` the
    /// most beats of a convolution's channels, the kernel's depth a tap. A job of any other shape runs on the vector unit,
    /// which holds [`CONV_PATH_VREGS`] registers; one whose loads, stores or instructions name a register past them faults.
    pub fn with_conv_path(self, lanes: usize, beats: usize) -> Streaming {
        assert!(self.vectors, "a convolution's path is the vector unit's side's");
        assert!([1, 2, 4, 8, 16].contains(&lanes), "a conv path of 1, 2, 4, 8 or 16 lanes, not {lanes}");
        assert!(beats >= 1, "a conv path's kernel holds a beat a tap at least");
        Streaming { conv_lanes: lanes, conv_beats: beats, ..self }
    }

    /// The `beats` of [`Streaming::with_conv_path`] for checkpoints of the specs `specs`: the most channels of any of
    /// their Gated-DeltaNet layers' convolutions (the keys twice and the values), sixteen a beat; 1 for none.
    pub fn conv_beats<'a>(specs: impl IntoIterator<Item = &'a ModelSpec>) -> usize {
        specs
            .into_iter()
            .flat_map(|spec| spec.layers.iter())
            .filter_map(|layer| match &layer.mixer {
                Mixer::GatedDelta(g) => Some((2 * g.k_heads + g.v_heads) * g.head_dim),
                _ => None,
            })
            .max()
            .map_or(1, |channels| channels.div_ceil(16))
    }

    /// The unit with the recurrence unit beside it.
    pub fn with_recurrence(self) -> Streaming {
        Streaming { recurrence: true, ..self }
    }

    /// The unit with its recurrence unit taking `width` sixteens of elements a clock, 1 or 2.
    pub fn with_recurrence_width(self, width: usize) -> Streaming {
        assert!(self.recurrence, "a recurrence unit's width needs the recurrence unit");
        assert!(width == 1 || width == 2, "the recurrence unit takes half a binary16 line a clock or a whole one, not {width} sixteens");
        Streaming { recurrence_width: width, ..self }
    }

    /// The unit with its recurrence unit's value heads on `lanes` head lanes, 1 to 4, each lane's copy of the log
    /// `log_lines` lines.
    pub fn with_recurrence_lanes(self, lanes: usize, log_lines: usize) -> Streaming {
        assert!(self.recurrence, "head lanes are the recurrence unit's");
        assert!((1..=4).contains(&lanes), "the recurrence unit runs on 1 to 4 head lanes, not {lanes}");
        assert!(lanes == 1 || log_lines >= 1, "head lanes read the log from copies of at least a line");
        Streaming { recurrence_lanes: lanes, log_lines: if lanes == 1 { 0 } else { log_lines }, ..self }
    }

    /// The unit with each head lane's flushing head split over `flushes` flush units by rows, 1, 2, 4 or 8.
    pub fn with_recurrence_flushes(self, flushes: usize) -> Streaming {
        assert!(self.recurrence_lanes > 1, "flush units are a head lane's");
        assert!(self.recurrence_width == 2, "a head lane's flush units take a binary16 line a clock");
        assert!([1, 2, 4, 8].contains(&flushes), "a head lane's flush over 1, 2, 4 or 8 units, not {flushes}");
        Streaming { recurrence_flushes: flushes, ..self }
    }

    /// The unit with its recurrence's coordinator and head lanes asking up to `lines` lines ahead of those they have taken,
    /// from [`super::gdn::LANE_CREDIT`] to [`super::gdn::LANE_CREDIT_MAX`].
    pub fn with_recurrence_credit(self, lines: usize) -> Streaming {
        use super::gdn::{LANE_CREDIT, LANE_CREDIT_MAX};
        assert!(self.recurrence_lanes > 1, "a read credit is the head lanes'");
        assert!(
            (LANE_CREDIT..=LANE_CREDIT_MAX).contains(&lines),
            "a head lane asks {LANE_CREDIT} to {LANE_CREDIT_MAX} lines ahead, not {lines}"
        );
        Streaming { recurrence_credit: lines, ..self }
    }

    /// The unit with the vector unit's side beside it, on the recurrence unit's port onto the channels.
    pub fn with_vectors(self) -> Streaming {
        assert!(self.recurrence, "the vector unit's side runs on the recurrence unit's port");
        assert_eq!(self.layout.line_bytes, 64, "the vector unit's side reads and writes the channels' 64-byte lines");
        Streaming { vectors: true, ..self }
    }

    /// The unit with its vector unit's side holding `rows` token-rows of a float matrix's inputs at once.
    pub fn with_float_rows(self, rows: usize) -> Streaming {
        assert!(self.vectors, "a float matrix's rows are the vector unit's side's");
        assert!(rows >= 1, "a float matrix's group of no rows");
        Streaming { float_rows: rows, ..self }
    }

    /// The unit with its vector unit's side running a grouped job's passes `passes` at a time, 1 to
    /// [`VECTOR_PASSES_MAX`]: a reduction's passes in subgroups of the add's latency and one, a slot of its round each.
    pub fn with_vector_passes(self, passes: usize) -> Streaming {
        assert!(self.vectors, "a group of passes is the vector unit's side's");
        assert!(
            (1..=VECTOR_PASSES_MAX).contains(&passes),
            "the vector unit runs 1 to {VECTOR_PASSES_MAX} passes at once, not {passes}"
        );
        Streaming { vector_passes: passes, ..self }
    }

    /// The unit with its vector unit's side's registers `depth` passes of the deepest vector job it runs whole deep, 1 to
    /// [`VECTOR_PASSES_MAX`]: a job whose vectors are that deep runs `depth` of its passes a group (up to the side's
    /// passes a group), its chunks and its float matrices' held inputs still a vector deep.
    pub fn with_vector_depth(self, depth: usize) -> Streaming {
        assert!(self.vectors, "a register's depth is the vector unit's side's");
        assert!(
            (1..=VECTOR_PASSES_MAX).contains(&depth),
            "the vector unit's side holds 1 to {VECTOR_PASSES_MAX} passes of its deepest job, not {depth}"
        );
        Streaming { vector_depth: depth, ..self }
    }

    /// The unit with the attention side beside the vector unit's side, on the recurrence unit's port.
    pub fn with_attention(self) -> Streaming {
        assert!(self.vectors, "the attention side runs beside the vector unit's side, on the recurrence unit's port");
        Streaming { attention: true, ..self }
    }

    /// The unit with `contexts` sequencer contexts, 1 or 2, sharing its units: every unit must be on the fast side (the
    /// attention side beside the rest), since only the fast side's units run beside one another, and a context's
    /// program counter crosses to the shell in 16 bits.
    pub fn with_contexts(self, contexts: usize) -> Streaming {
        assert!(self.attention, "contexts share the fast side's units, so every unit must be on it: the attention side too");
        assert!(contexts == 1 || contexts == 2, "an engine of 1 or 2 contexts, not {contexts}");
        Streaming { contexts, ..self }
    }

    /// The unit with its fast side's f32 adds, and its multiplies given a product block, on the floating-point DSP block
    /// `hard`: refused when a wrapper cannot give the soft unit's latency on it, or the engine's add, or the multiply of
    /// `hard`'s unit ([`HardFloat::product_target`]); a build of the card's products has its product block.
    pub fn with_hard_float(self, hard: HardFloat) -> Result<Streaming, HardError> {
        assert!(hard.fp_unit == FpUnit::Soft || hard.product.is_some(), "a build of the card's products builds them on its block");
        if let Some(product) = &hard.product {
            plan_for(fp_hard::Op::Multiply, product, FMUL_LATENCY, &hard.product_target())?;
        }
        plan(fp_hard::Op::Add, &hard.sum, FADD_LATENCY)?;
        Ok(Streaming { hard_float: Some(hard), ..self })
    }

    /// The unit folding a job's sums into its output slot over `lanes` fold lanes, 1, 2, 4, 8 or 16: a step whose residual
    /// adds and gated products are folded into its matrix jobs ([`super::super::machine::fold`]) runs on it.
    pub fn with_folds(self, lanes: usize) -> Streaming {
        assert!([1, 2, 4, 8, 16].contains(&lanes), "a fold over 1, 2, 4, 8 or 16 lanes, not {lanes}");
        Streaming { folds: lanes, ..self }
    }

    /// The head lanes making each recurrence job's gated norm as its read-outs are made ([`Streaming::delta_norm`]): a
    /// step whose gated norms are folded into its recurrence jobs runs on it.
    pub fn with_delta_norm(self) -> Streaming {
        assert!(self.recurrence_lanes > 1, "a recurrence job's gated norm is made on its head lanes");
        Streaming { delta_norm: true, ..self }
    }

    /// The name the build and the programs written for it give the unit (`LOGOS_D5005_STREAMING`, `engine.txt`'s
    /// `streaming`): its column set, the row pairs a channel's stripe holds, whether it folds, and whether its recurrence
    /// makes its gated norms.
    pub fn signature(&self) -> String {
        format!(
            "{}/{}{}{}",
            self.set.signature(),
            self.pairs(),
            if self.folds > 0 { "+folds" } else { "" },
            if self.delta_norm { "+norm" } else { "" }
        )
    }

    /// The row pairs a channel's share of a stripe holds: the column array's cores a channel.
    pub fn pairs(&self) -> u64 {
        self.layout.stripe.expect("a stripe")
    }
}

/// The function units the vector unit's sixteen lanes share: a quarter of a unit a lane, the vector unit's largest
/// part (each unit an exp, a reciprocal, an ln and a reciprocal square root) in a quarter of the area.
pub const FUNCTION_UNITS: usize = 4;

/// The most passes a vector unit's side runs at once ([`Streaming::with_vector_passes`]): each pass holds its own scalar
/// registers and running sums, so the latency every instruction pays is shared by the group.
pub const VECTOR_PASSES_MAX: usize = 32;

/// The vector registers of the vector unit of a side whose convolutions run on their path ([`Streaming::with_conv_path`]),
/// half of the sixteen a job's fields can name: every job the machine lowers but a convolution names fewer.
pub const CONV_PATH_VREGS: usize = 8;

impl EngineParams {
    /// The name the build and its programs give its streaming unit, the step program refusing a program of another: the
    /// unit's own ([`Streaming::signature`]), and `+behind<T>` when its recurrence replays logs a window of `T` tokens
    /// behind, its lanes built for that window; empty for an engine that streams nothing.
    pub fn streaming_signature(&self) -> String {
        self.streaming.as_ref().map_or(String::new(), |s| format!("{}{}", s.signature(), behind(self.delta.map(|(_, f)| f))))
    }

    /// The engine `jobs` over `layout` need, the store laid out as `store`.
    pub fn of(layout: &Layout, store: &StoreLayout, jobs: &[Job]) -> EngineParams {
        let mut formats: Vec<(GgmlType, usize)> = Vec::new();
        let (mut vector_rows, mut side_rows, mut pair_rows) = (1, 1, 1);
        for job in jobs {
            match job {
                Job::Matvec { tensor, .. } => {
                    let t = &layout.tensors[*tensor];
                    let blocks = t.dims[0] as usize / t.ty.block_len();
                    match formats.iter_mut().find(|(ty, _)| *ty == t.ty) {
                        Some(f) => f.1 = f.1.max(blocks),
                        None => formats.push((t.ty, blocks)),
                    }
                }
                Job::Vector(v) => {
                    let rows = v.loads.iter().map(|l| l.len.div_ceil(LANES)).fold(1, usize::max);
                    vector_rows = vector_rows.max(rows);
                    if !v.chunked() {
                        side_rows = side_rows.max(rows);
                    }
                    if v.program.iter().any(|op| matches!(op, Op::Hadamard { .. } | Op::Rotate { .. })) {
                        pair_rows = pair_rows.max(rows);
                    }
                }
                _ => {}
            }
        }
        formats.sort_by_key(|(ty, _)| ty.id());
        EngineParams {
            rows: store.rows,
            vector_rows,
            side_rows,
            pair_rows,
            store_beats: store.beats as usize,
            job_words: encode(jobs).len().max(1),
            tensors: layout.tensors.len(),
            layers: layout.blocks.len(),
            ropes: layout.rotary.len(),
            formats,
            delta: layout.blocks.iter().find_map(|b| match b {
                BlockState::Delta { width, .. } => Some((*width, layout.numerics.flush)),
                _ => None,
            }),
            attend: layout.blocks.iter().find_map(|b| match b {
                BlockState::Attention { width, value_width, .. } => {
                    Some((*width, *value_width, layout.ctx, layout.numerics.attention == crate::spatial::serving::AttentionMath::Array))
                }
                _ => None,
            }),
            function_units: FUNCTION_UNITS,
            folds: jobs.iter().any(|j| matches!(j, Job::Matvec { fold, .. } if *fold != Fold::Put)),
            streaming: None,
        }
    }

    /// One engine that runs every engine of `all`: the most token-rows, vector rows, store beats, job words and table
    /// entries any needs, every block format any reads at the most blocks a row of it holds in any, the most positions
    /// any attends over. Engines whose recurrence or attention units differ in shape — head widths, a log's tokens,
    /// where the attention math runs —, whose vector units share a different number of function units, or that run
    /// their matvec jobs on different streaming units have no union.
    pub fn union(all: &[EngineParams]) -> Result<EngineParams, String> {
        let (first, rest) = all.split_first().ok_or("no engines")?;
        let mut u = first.clone();
        for p in rest {
            if p.function_units != u.function_units {
                return Err(format!("{} and {} function units", u.function_units, p.function_units));
            }
            if p.streaming != u.streaming {
                return Err(format!("streaming units {:?} and {:?}", u.streaming, p.streaming));
            }
            u.delta = match (u.delta, p.delta) {
                (Some(a), Some(b)) if a != b => return Err(format!("recurrence units {a:?} and {b:?}")),
                (a, b) => a.or(b),
            };
            u.attend = match (u.attend, p.attend) {
                (Some((hd, vd, ctx, array)), Some((hd2, vd2, ctx2, array2))) => {
                    if (hd, vd, array) != (hd2, vd2, array2) {
                        return Err(format!("attention units {:?} and {:?}", (hd, vd, array), (hd2, vd2, array2)));
                    }
                    Some((hd, vd, ctx.max(ctx2), array))
                }
                (a, b) => a.or(b),
            };
            u.rows = u.rows.max(p.rows);
            u.vector_rows = u.vector_rows.max(p.vector_rows);
            u.side_rows = u.side_rows.max(p.side_rows);
            u.pair_rows = u.pair_rows.max(p.pair_rows);
            u.store_beats = u.store_beats.max(p.store_beats);
            u.job_words = u.job_words.max(p.job_words);
            u.tensors = u.tensors.max(p.tensors);
            u.layers = u.layers.max(p.layers);
            u.ropes = u.ropes.max(p.ropes);
            u.folds |= p.folds;
            for &(ty, blocks) in &p.formats {
                match u.formats.iter_mut().find(|(t, _)| *t == ty) {
                    Some(f) => f.1 = f.1.max(blocks),
                    None => u.formats.push((ty, blocks)),
                }
            }
        }
        u.formats.sort_by_key(|(ty, _)| ty.id());
        Ok(u)
    }
}

/// Verilog sources as one: every module once — a module two sources define being the same text in each — with the
/// text before it, the macros it reads among it.
pub fn compose(sources: &[&str]) -> String {
    let mut seen: HashMap<String, String> = HashMap::new();
    let mut out = String::new();
    for src in sources {
        let mut rest: &str = src;
        loop {
            let start = rest.match_indices("module ").map(|(i, _)| i).find(|&i| i == 0 || rest.as_bytes()[i - 1] == b'\n');
            let Some(i) = start else {
                if !rest.trim().is_empty() && !out.contains(rest.trim()) {
                    out.push_str(rest);
                }
                break;
            };
            let end = i + rest[i..].find("endmodule").expect("a module's end") + "endmodule".len();
            let name: String = rest[i + 7..].chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            let body = &rest[i..end];
            match seen.get(&name) {
                Some(prior) => assert_eq!(prior, body, "two sources define the module {name} two ways"),
                None => {
                    seen.insert(name, body.to_string());
                    out.push_str(&rest[..end]);
                    out.push('\n');
                }
            }
            rest = &rest[end..];
        }
    }
    out
}

/// The instance statements under `top` in `verilog`, counted through the hierarchy: every module `top` instantiates,
/// directly or through the modules it instantiates, with the number of statements that reach it. A statement inside a
/// generate loop counts once.
pub fn census(verilog: &str, top: &str) -> BTreeMap<String, usize> {
    let mut bodies: HashMap<&str, &str> = HashMap::new();
    let mut rest = verilog;
    while let Some(i) = rest.match_indices("module ").map(|(i, _)| i).find(|&i| i == 0 || rest.as_bytes()[i - 1] == b'\n') {
        let end = i + rest[i..].find("endmodule").expect("a module's end");
        let name = &rest[i + 7..];
        let name = &name[..name.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(name.len())];
        bodies.insert(name, &rest[i..end]);
        rest = &rest[end + "endmodule".len()..];
    }
    fn walk(module: &str, reach: usize, bodies: &HashMap<&str, &str>, out: &mut BTreeMap<String, usize>) {
        let Some(body) = bodies.get(module) else { return };
        for line in body.lines() {
            let t = line.trim_start();
            let child = &t[..t.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(t.len())];
            let after = t[child.len()..].trim_start();
            if bodies.contains_key(child) && after.starts_with(|c: char| c == '#' || c.is_alphabetic() || c == '_') {
                *out.entry(child.to_string()).or_default() += reach;
                walk(child, reach, bodies, out);
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(top, 1, &bodies, &mut out);
    out
}

/// The engine for `p`, with the vector unit, the array's columns for every format `p` names, the activation
/// quantizer they and the query heads share and the f32 units they are built from; a hybrid engine's matvec jobs on its
/// streaming unit instead of the columns, the quantizer the query heads' alone.
pub fn engine_verilog(p: &EngineParams) -> String {
    assert!(
        !p.folds || p.streaming.as_ref().is_some_and(|s| s.folds > 0),
        "a step that folds its matrices' sums into their output slots runs on a streaming unit that folds them (Streaming::with_folds)"
    );
    let columns = if p.streaming.is_some() { &[][..] } else { &p.formats[..] };
    let mut block_lens: Vec<usize> = columns.iter().map(|(ty, _)| ty.block_len()).collect();
    // A query head on the array is quantized as one block of its width.
    if let Some((hd, _, _, true)) = p.attend {
        block_lens.push(hd);
    }
    block_lens.sort();
    block_lens.dedup();
    let widest = p.formats.iter().map(|(ty, _)| 8 * ty.block_bytes()).max().unwrap_or(8);
    let mut cases = String::new();
    let mut units = String::new();
    let mut ready = String::from("1'b0");
    let mut col_busy = String::from("1'b0");
    let mut out_mux = String::new();
    for (i, (ty, blocks)) in columns.iter().enumerate() {
        let name = ty.name().to_lowercase();
        let (len, bytes) = (ty.block_len(), ty.block_bytes());
        writeln!(cases, "      8'd{}: begin mv_known = 1'b1; mv_fmt = 4'd{i}; mv_blen = 16'd{len}; mv_bbytes = 16'd{bytes}; end", ty.id()).unwrap();
        writeln!(
            units,
            r#"  wire mv_busy_{i}, mv_ready_{i}, mv_ov_{i};
  wire [15:0] mv_op_{i};
  wire [7:0] mv_ot_{i};
  wire [31:0] mv_o1_{i}, mv_o2_{i};
  logos_matvec_engine_{name} #(.T_MAX(ROW_MAX), .NB_MAX({blocks})) col_{i}(.clk(clk), .rst(rst), .start(mv_start && mv_fmt == 4'd{i}),
    .pairs(mv_pairs[15:0]), .blocks(mv_blocks[15:0]), .tokens(rows[7:0]), .two(two_words), .busy(mv_busy_{i}),
    .act_wr(act_wr && mv_fmt == 4'd{i}), .act_t(act_t), .act_b(act_b), .act_codes(aq_codes[{codes_top}:0]), .act_m(aq_m), .act_e(aq_e),
    .w_valid(w_valid && mv_fmt == 4'd{i}), .w_ready(mv_ready_{i}), .w_blk1(w_blk1[{top}:0]), .w_blk2(w_blk2[{top}:0]),
    .o_valid(mv_ov_{i}), .o_pair(mv_op_{i}), .o_t(mv_ot_{i}), .o_val1(mv_o1_{i}), .o_val2(mv_o2_{i}));"#,
            top = 8 * bytes - 1,
            codes_top = 16 * len - 1
        )
        .unwrap();
        ready = format!("{ready} | (mv_fmt == 4'd{i} && mv_ready_{i})");
        col_busy = format!("{col_busy} | mv_busy_{i}");
        writeln!(out_mux, "    if (mv_fmt == 4'd{i}) begin o_v = mv_ov_{i}; o_p = mv_op_{i}; o_t = mv_ot_{i}; o_1 = mv_o1_{i}; o_2 = mv_o2_{i}; end").unwrap();
    }
    let attention_side = p.streaming.as_ref().is_some_and(|s| s.attention);
    let (quantizers, feeds) = match block_lens.last() {
        Some(_) if attention_side => ("  wire aq_busy = 1'b0;\n".to_string(), String::new()),
        Some(&lanes) => {
            assert!(block_lens.iter().all(|len| len % 16 == 0), "blocks of {block_lens:?} are not whole beats");
            let at_feeds = matches!(p.attend, Some((_, _, _, true)));
            let shared = if p.streaming.is_some() { "the query heads share" } else { "every weight block and query head shares" };
            (
                format!(
                    r#"  // The activation quantizer {shared}, fed a beat a cycle from the store.
  wire aq_in, aq_clear, aq_go, aq_busy;
  wire [7:0] aq_m;
  wire signed [11:0] aq_e;
  wire [{}:0] aq_codes;
  logos_act_stream #(.LANES({lanes})) aq(.clk(clk), .rst(rst), .clear(aq_clear), .in_v(aq_in), .in_beat(st_rd), .go(aq_go),
    .two(two_words), .busy(aq_busy), .m(aq_m), .e(aq_e), .codes(aq_codes));
"#,
                    16 * lanes - 1
                ),
                if p.streaming.is_some() {
                    "  // A query head's beats while it loads, its codes once the last is in.\n  assign aq_in = state == AT_Q && at_lj != AH_BEATS;\n  \
                     assign aq_clear = state == AT_Q && at_lj == 32'd0;\n  assign aq_go = state == AT_Q && at_lj == AH_BEATS;\n"
                        .to_string()
                } else {
                    format!(
                    r#"  // A weight block's beats while it loads, its codes once the last is in; a query head's the same.
  wire mv_aq = state == MV_ACT && mv_known && {{8'd0, act_t}} != {{8'd0, rows}};
  wire mv_aq_in = mv_aq && aj < {{16'd0, mv_blen}} >> 4;
  assign aq_in = mv_aq_in{at_in};
  assign aq_clear = (mv_aq_in && aj == 32'd0){at_clear};
  assign aq_go = (mv_aq && aj == {{16'd0, mv_blen}} >> 4){at_go};
"#,
                    at_in = if at_feeds { " || (state == AT_Q && at_lj != AH_BEATS)" } else { "" },
                    at_clear = if at_feeds { " || (state == AT_Q && at_lj == 32'd0)" } else { "" },
                    at_go = if at_feeds { " || (state == AT_Q && at_lj == AH_BEATS)" } else { "" },
                    )
                },
            )
        }
        None => ("  wire aq_busy = 1'b0;\n".to_string(), String::new()),
    };
    let fast_recurrence = p.streaming.as_ref().is_some_and(|s| s.recurrence);
    assert!(!fast_recurrence || p.delta.is_some(), "a recurrence unit runs a model's Gated-DeltaNet blocks");
    if let (true, Some((hd, _))) = (fast_recurrence, p.delta) {
        assert!(hd * 2 % 64 == 0, "a recurrence unit walks a head's memory in whole lines, and a head {hd} wide is not whole lines in binary16");
    }
    let vectors = p.streaming.as_ref().is_some_and(|s| s.vectors);
    // A vectored engine whose memories are lazily flushed hands every token-row of them to the recurrence unit and every
    // vector job to the vector unit's side, so its core runs no vector instruction and updates no memory itself.
    let handed_off = vectors && p.delta.is_none_or(|(_, f)| recurrence_log(f).0 > 1);
    if let Some((_, StateFlush::Trailing { .. })) = p.delta {
        assert!(fast_recurrence && p.streaming.as_ref().is_some_and(|s| s.recurrence_lanes > 1), "a trailing flush runs on the recurrence's head lanes alone");
    }
    let normed = p.streaming.as_ref().is_some_and(|s| s.delta_norm);
    if normed {
        assert!(
            p.delta.is_some_and(|(_, f)| recurrence_log(f).0 > 1),
            "a recurrence makes its gated norms on its head lanes, which run the memories it flushes lazily alone"
        );
    }
    let delta = p.delta.map_or_else(UnitVerilog::default, |(hd, f)| {
        let t = recurrence_log(f).0;
        if handed_off {
            delta_handed_off(hd, t, normed)
        } else if fast_recurrence {
            let unit = delta_on_the_fast_side(delta_verilog(hd, t));
            if normed { delta_normed_core(unit) } else { unit }
        } else {
            delta_verilog(hd, t)
        }
    });
    let attention_side = p.streaming.as_ref().is_some_and(|s| s.attention);
    assert!(!attention_side || matches!(p.attend, Some((_, _, _, true))), "the attention side attends over an int8 cache on the array");
    let attend_unit = match p.attend {
        Some(_) if attention_side => attend_handed_off(),
        Some((hd, vd, ctx, array)) => attend_unit_verilog(hd, vd, ctx, array),
        None => UnitVerilog::default(),
    };
    let delta_pulses = format!(
        "{}{}",
        match (p.delta.is_some(), fast_recurrence, handed_off) {
            (true, false, _) => "dre_v <= 1'b0; dlr_v <= 1'b0; dlr_log_wr <= 1'b0; ",
            (true, true, false) => "dre_v <= 1'b0; sm_gdn_go <= 1'b0; ",
            (true, true, true) => "sm_gdn_go <= 1'b0; ",
            (false, _, _) => "",
        },
        match (p.attend.is_some(), attention_side) {
            (true, false) => "at_start <= 1'b0;",
            (true, true) => "sm_att_go <= 1'b0;",
            (false, _) => "",
        }
    );
    let delta_reads = match (p.delta.is_some(), fast_recurrence, handed_off) {
        (true, false, _) => {
            "if (dl_rd) begin s_rd_reg = dl_rd_reg; s_rd_row = dl_rd_row; end\n    if (state == DL_KQ_WAIT) s_rd_s = 4'd0;\n    if (state == DL_FEXP_WAIT) s_rd_s = 4'd1;"
        }
        (true, true, false) => "if (state == DL_KQ_WAIT) s_rd_s = 4'd0;",
        (_, _, true) | (false, _, _) => "",
    };
    let mv = match &p.streaming {
        None => MatvecText::reference(),
        Some(s) => MatvecText::hybrid(s),
    };
    let template = if vectors { vector_jobs_on_the_fast_side(ENGINE_VERILOG, handed_off) } else { ENGINE_VERILOG.to_string() };
    let engine = template
        .replace("@PORTS@", &mv.ports)
        .replace("@STORE_RAM@", &mv.store_ram)
        .replace("@MATVEC_DECLS@", &mv.decls)
        .replace("@MATVEC_STATES@", &mv.states)
        .replace("@MATVEC_PULSES@", &mv.pulses)
        .replace("@MATVEC_CASES@", &mv.cases)
        .replace("@MATVEC_READS@", &mv.reads)
        .replace("@STORE_WRITE_NOTE@", &mv.write_note)
        .replace("@MATVEC_WRITES@", &mv.writes)
        .replace("@DELTA_DECLS@", &delta.decls)
        .replace("@DELTA_CASES@", &delta.cases)
        .replace("@STORE_READS@", &format!("{}{}", delta.reads, attend_unit.reads))
        .replace("@STORE_WRITES@", &format!("{}{}", delta.writes, attend_unit.writes))
        .replace("@DELTA_PULSES@", &delta_pulses)
        .replace("@ATTEND_DECLS@", &attend_unit.decls)
        .replace("@ATTEND_CASES@", &attend_unit.cases)
        .replace("@ATTEND_UNIT@", if p.attend.is_some() { "4" } else { "15" })
        .replace("@DELTA_READS@", delta_reads)
        .replace("@DELTA_UNIT@", if p.delta.is_some() { "3" } else { "15" })
        .replace("@ROWS@", &p.rows.to_string())
        .replace("@VROWS@", &p.vector_rows.to_string())
        .replace("@FU@", &p.function_units.to_string())
        .replace("@STORE@", &p.store_beats.to_string())
        .replace("@JOBS@", &p.job_words.to_string())
        .replace("@TENSORS@", &p.tensors.max(1).to_string())
        .replace("@LAYERS@", &p.layers.max(1).to_string())
        .replace("@ROPES@", &p.ropes.max(1).to_string())
        .replace("@WIDEST@", &(widest - 1).to_string())
        .replace("@FORMAT_CASES@", &cases)
        .replace("@QUANTIZERS@", &quantizers)
        .replace("@QUANTIZER_FEEDS@", &feeds)
        .replace("@COLUMNS@", &units)
        .replace("@READY@", &ready)
        .replace("@COLUMN_BUSY@", &col_busy)
        .replace("@OUT_MUX@", &out_mux);
    let simd = super::simd::simd_verilog();
    let matvec = if p.formats.is_empty() || p.streaming.is_some() { String::new() } else { super::matvec::matvec_verilog() };
    let gdn = if p.delta.is_some() { super::gdn::gdn_verilog() } else { String::new() };
    let attend = if p.attend.is_some() { super::attend::attend_verilog() } else { String::new() };
    let ram = super::ram::ram_verilog(&[64, 128, 512, 608]);
    let act = super::act_quant::act_quant_verilog();
    let core = engine.replacen("module logos_engine #(", "module logos_engine_core #(", 1);
    // A core that reads the token-row store for the host alone leaves every other read of it to the fast side, which
    // reads it through the pipelined bank: each reader takes its word STORE_READ_LATENCY clocks after its address, and
    // the host's read, which takes its word a compute step after asking, the same.
    let srl = if p.streaming.is_some() && reads_the_store_for_the_host_alone(&engine) { STORE_READ_LATENCY } else { 1 };
    assert!(
        p.streaming.as_ref().is_none_or(|s| s.contexts == 1) || !core.contains("wq_v <= 1'b1"),
        "an engine of contexts shares the card's port for reads alone, and this core writes the card"
    );
    let enabled = enable(&compose(&[&ram, &simd, &matvec, &act, &gdn, &attend, &core]));
    let fast = match &p.streaming {
        Some(s) => {
            let recurrence = if s.recurrence {
                let rows = super::gdn::rows_verilog();
                let rows = if s.delta_norm { super::gdn::delta_normed_rows(&rows) } else { rows };
                let rows = if s.contexts > 1 { gdn_rows_contexts(&rows) } else { rows };
                format!("\n{}\n{rows}\n{}", super::gdn::gdn_stream_verilog(), super::super::physical::fast_port_verilog())
            } else {
                String::new()
            };
            let recurrence = if s.recurrence_lanes > 1 {
                let lanes = match p.delta {
                    Some((_, StateFlush::Trailing { tokens })) => super::gdn::lanes_trailing_verilog(tokens as usize),
                    _ => super::gdn::lanes_verilog(),
                };
                let lanes = match p.delta {
                    Some((hd, _)) if s.delta_norm => super::gdn::delta_normed_lanes(&lanes, hd, s.hard_float.map_or(FpUnit::Soft, |h| h.fp_unit)),
                    _ => lanes,
                };
                format!("{recurrence}\n{lanes}")
            } else {
                recurrence
            };
            let recurrence = if s.recurrence_lanes > 1 || s.contexts > 1 {
                format!("{recurrence}\n{}", super::super::physical::channel_mux_verilog())
            } else {
                recurrence
            };
            let recurrence = if s.contexts > 1 {
                format!("{recurrence}\n{}", super::super::physical::read_credit_verilog())
            } else {
                recurrence
            };
            let vector_side = || {
                let text = super::vector_side::vector_side_verilog();
                let text = if s.conv_lanes > 0 {
                    format!("{}\n{}", vector_side_conv(&text, s.conv_lanes, s.conv_beats), super::vector_side::conv_path_verilog())
                } else {
                    text
                };
                if s.contexts > 1 {
                    vector_side_contexts(&text)
                } else {
                    text
                }
            };
            let recurrence = if s.vectors { format!("{recurrence}\n{}", vector_side()) } else { recurrence };
            let recurrence = if s.attention { format!("{recurrence}\n{}", super::attend::attend_side_verilog()) } else { recurrence };
            let bank = if srl > 1 { super::ram::store_verilog() } else { super::ram::bram_re_verilog(&[512]) };
            format!(
                "{}\n{bank}\n{}\n{}{recurrence}",
                super::super::physical::pipe_verilog(),
                super::streaming::streaming_verilog_folding(std::slice::from_ref(&s.set), &s.layout, s.folds),
                fast_store(&fast_verilog(p, s), srl)
            )
        }
        None => String::new(),
    };
    let engine = format!("{}{}", super::ram::RAMSTYLE, prune(&compose(&[&enabled, &fast, &bridge_of(&crossing_ports(p)), &top_verilog(p)]), "logos_engine"));
    match p.streaming.as_ref().and_then(|s| s.hard_float) {
        Some(hard) => hard_float_verilog(&engine, &hard),
        None => engine,
    }
}

/// The engine `verilog` ([`engine_verilog`]'s text) with its f32 adds, and its multiplies given a product block, on the
/// floating-point DSP block `hard`: `logos_fadd_p` (and `logos_fmul_p`) replaced by the block's wrappers, the same ports
/// and latencies. Every instance of either is the fast side's; refuses an engine whose core reaches one, the core
/// stepping at its enables where the wrappers run free.
pub fn hard_float_verilog(verilog: &str, hard: &HardFloat) -> String {
    let core = census(verilog, "logos_engine_core_ce");
    for unit in ["logos_fmul_p", "logos_fadd_p"] {
        assert!(!core.contains_key(unit), "the engine's core reaches `{unit}`, which the floating-point block's wrapper runs free of its enables");
    }
    let wrapper = |op, unit, block, latency| hard_verilog(op, unit, block, latency).unwrap_or_else(|e| panic!("{unit} on the block: {e}"));
    modules(verilog)
        .into_iter()
        .map(|(name, text)| {
            let head = &text[..text.find(&format!("module {name}")).unwrap_or(text.len())];
            match name.as_str() {
                "logos_fmul_p" => match &hard.product {
                    Some(product) => format!(
                        "{head}{}",
                        hard_verilog_for(fp_hard::Op::Multiply, "logos_fmul_p", product, FMUL_LATENCY, &hard.product_target())
                            .unwrap_or_else(|e| panic!("logos_fmul_p on the block: {e}"))
                    ),
                    None => format!("{text}\n"),
                },
                "logos_fadd_p" => format!("{head}{}", wrapper(fp_hard::Op::Add, "logos_fadd_p", &hard.sum, FADD_LATENCY)),
                _ => format!("{text}\n"),
            }
        })
        .collect()
}

/// The floating-point DSP block a simulation of an engine on `hard` stands in for the device's (`fourteennm_fp_mac`,
/// [`crate::codegen_sva::fp_oracle::hard_model_verilog`]): the measured semantics of its multiply and add at their
/// measured depths, the multiply-add as deep as the deeper of them. With the multiplies soft no instance takes the
/// block's multiply, and the stand-in's is the add's.
pub fn hard_float_model(hard: &HardFloat) -> String {
    let (m, a) = (hard.product.unwrap_or(hard.sum), hard.sum);
    hard_model_verilog(&m.semantics, &a.semantics, Depths { full: [m.full, a.full, m.full.max(a.full)], least: [m.least, a.least] })
}

/// Whether an engine's `core` (its template's text, every unit's spliced in) reads the token-row store for the host alone:
/// its one read address the host's.
fn reads_the_store_for_the_host_alone(core: &str) -> bool {
    core.matches("st_ra = ").count() == 1 && core.contains("      default: st_ra = host_raddr;\n")
}

/// The fast side `text` ([`fast_verilog`]'s) whose token-row store reads in `srl` clocks: at one clock the text itself; at
/// [`STORE_READ_LATENCY`] each bank the pipelined `logos_store_w512` and every reader of it — the streaming unit, the
/// recurrence, vector and attention sides — given the latency (`SRL`).
fn fast_store(text: &str, srl: usize) -> String {
    if srl == 1 {
        return text.to_string();
    }
    assert_eq!(srl, STORE_READ_LATENCY, "a store reads in one clock or in the pipelined bank's");
    assert_eq!(text.matches("logos_bram_re_w512 #(").count(), 1, "the fast side's one store instance");
    let mut t = text.replace("logos_bram_re_w512 #(", "logos_store_w512 #(");
    // Each reader's latency its parameters' last, the parameters it had before kept as they were.
    let given = |t: &mut String, from: &str| -> bool {
        let Some(at) = t.find(from) else { return false };
        let open = at + t[at..].find("#(").expect("an instance's parameters") + 1;
        let mut depth = 0;
        let close = open
            + t[open..]
                .char_indices()
                .find(|&(_, c)| {
                    depth += match c {
                        '(' => 1,
                        ')' => -1,
                        _ => 0,
                    };
                    depth == 0
                })
                .expect("an instance's parameters closed")
                .0;
        t.insert_str(close, &format!(", .SRL({srl})"));
        true
    };
    assert!(given(&mut t, "  logos_streaming_matvec_"), "the fast side's streaming unit");
    let mut readers = Vec::new();
    for reader in ["logos_vector_side", "logos_gdn_side", "logos_gdn_lanes", "logos_attend_side"] {
        let from = format!("  {reader} #(");
        assert!(t.matches(&from).count() <= 1, "one {reader} on the fast side");
        if given(&mut t, &from) {
            readers.push(reader);
        }
    }
    for reader in ["logos_vector_side", "logos_attend_side"] {
        assert!(readers.contains(&reader), "a fast side whose core reads the store for the host alone runs its {reader}");
    }
    assert!(
        readers.contains(&"logos_gdn_side") || readers.contains(&"logos_gdn_lanes"),
        "a fast side whose core reads the store for the host alone runs its recurrence"
    );
    t
}

/// The ports past the template's a hybrid engine's crossing takes from the shell for its fast side: the card port's
/// map ([`super::super::physical`]), which a recurrence unit's port places its lines by; none for any other engine.
fn bridged_map(p: &EngineParams) -> Vec<(bool, String, String)> {
    if !p.streaming.as_ref().is_some_and(|s| s.recurrence) {
        return Vec::new();
    }
    vec![
        (true, "[31:0]".to_string(), "map_space".to_string()),
        (true, format!("[{}:0]", super::stream::AW - 1), "map_base".to_string()),
        (true, "[4:0]".to_string(), "map_granule_log2".to_string()),
    ]
}


/// The engine `verilog` ([`engine_verilog`]'s text) as a simulator runs it: the core's clock gated to the user cycles
/// the core steps in (`fedge`) or reads its memories in (`cr`), the one difference. Every register of the core steps
/// under `step_en` and every memory reads under `read_en` and writes under `ce`, so a rising edge of the user clock with
/// neither high changes nothing in the core, and without those edges a simulator evaluates the core's logic in two user
/// cycles of an engine cycle instead of in every one. The gate is taken at the falling edge, so the gated clock rises
/// only with the user clock. Refuses an engine whose core holds state any other way, or enables an instance by anything
/// but its own `step_en` and `read_en`. A simulation model only: a PR persona gates no clock, every global network
/// being the FIM's.
pub fn simulation_twin(verilog: &str) -> String {
    let mut reached: Vec<String> = census(verilog, "logos_engine_core_ce").into_keys().collect();
    reached.push("logos_engine_core_ce".to_string());
    for (name, text) in modules(verilog).iter().filter(|(n, _)| reached.contains(n)) {
        let body = &text[text.find(&format!("module {name}")).expect("its header")..];
        let always = body.matches("always @(").count();
        if name.starts_with("logos_ram_ce_w") {
            let (read, write) = ("  always @(posedge clk) if (cr) rd <= mem[ra];\n", "  always @(posedge clk)\n    if (ce && we) begin\n");
            assert!(always == 2 && body.contains(read) && body.contains(write), "`{name}` reads under cr and writes under ce alone");
        } else if body.contains("(input clk, input step_en, input read_en, input rst,") {
            assert_eq!(body.matches("always @(posedge clk) if (step_en) begin").count(), always, "`{name}`'s every block steps under step_en");
        } else {
            assert!(!body.contains("posedge") && !body.contains("negedge") && !body.contains("input clk"), "`{name}` holds no state of its own");
        }
        assert_eq!(body.matches(".clk(").count(), body.matches(".clk(clk)").count(), "`{name}` clocks its instances by its own clock");
        let enabled = body.matches("(.clk(clk), .ce(step_en), .cr(read_en), ").count()
            + body.matches("(.clk(clk), .step_en(step_en), .read_en(read_en), ").count();
        assert_eq!(body.matches(".clk(clk)").count(), enabled, "`{name}` enables its every instance by its own step_en and read_en");
    }
    // Every core's instance (one, or one a context), each clocked by the one gate.
    let instances: Vec<String> = verilog
        .match_indices("  logos_engine_core_ce ")
        .map(|(at, head)| {
            let rest = &verilog[at + head.len()..];
            let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            assert!(
                rest[name.len()..].starts_with("(.clk(fclk), .step_en(fedge), .read_en(cr),"),
                "the core's instance `{name}` clocked by the user clock, stepping at fedge and reading at cr"
            );
            name
        })
        .collect();
    assert!(instances.first().is_some_and(|n| n == "core" || n == "core0"), "the core's instances: {instances:?}");
    assert!(instances.len() == 1 || (0..instances.len()).all(|k| instances[k] == format!("core{k}")), "the cores' instances: {instances:?}");
    let mut twin = verilog.to_string();
    for (k, name) in instances.iter().enumerate() {
        let from = format!("  logos_engine_core_ce {name}(.clk(fclk), .step_en(fedge), .read_en(cr),");
        let gate = if k == 0 {
            "  // The simulator's twin: the core's clock without the user cycles it neither steps nor reads its memories in.\n  \
             reg core_gate = 1'b0;\n  always @(negedge fclk) core_gate <= fedge || cr;\n  wire core_clk = fclk && core_gate;\n"
        } else {
            ""
        };
        twin = twin.replacen(&from, &format!("{gate}  logos_engine_core_ce {name}(.clk(core_clk), .step_en(fedge), .read_en(cr),"), 1);
    }
    twin
}

/// `verilog` with only the modules `top` instantiates, through its hierarchy, and `top` itself: of every other module
/// only the text before it, the macros it may define.
fn prune(verilog: &str, top: &str) -> String {
    let mut reached: Vec<String> = census(verilog, top).into_keys().collect();
    reached.push(top.to_string());
    modules(verilog)
        .into_iter()
        .map(|(name, text)| {
            if name.is_empty() || reached.contains(&name) {
                format!("{text}\n")
            } else {
                text[..text.find(&format!("module {name}")).expect("its header")].to_string()
            }
        })
        .collect()
}

/// Every module of `verilog` and its text, the text before it with it.
fn modules(verilog: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = verilog;
    loop {
        let start = rest.match_indices("module ").map(|(i, _)| i).find(|&i| i == 0 || rest.as_bytes()[i - 1] == b'\n');
        let Some(i) = start else {
            if !rest.trim().is_empty() {
                out.push((String::new(), rest.to_string()));
            }
            return out;
        };
        let end = i + rest[i..].find("endmodule").expect("a module's end") + "endmodule".len();
        let name: String = rest[i + 7..].chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        out.push((name, rest[..end].to_string()));
        rest = &rest[end..];
    }
}

/// `text` with every instance `pattern` begins renamed by `to` and given the enables, `enables` after its clock's
/// connection, the first `(.clk(clk), ` after it, within its statement.
fn enable_instances(text: &str, pattern: &str, to: &str, enables: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find(pattern) {
        out.push_str(&rest[..i]);
        out.push_str(to);
        rest = &rest[i + pattern.len()..];
        let clk = rest.find("(.clk(clk), ").expect("an instance's clock");
        assert!(!rest[..clk].contains(';'), "the clock of the instance `{pattern}` in its statement");
        out.push_str(&rest[..clk]);
        out.push_str("(.clk(clk), ");
        out.push_str(enables);
        rest = &rest[clk + "(.clk(clk), ".len()..];
    }
    out.push_str(rest);
    out
}

/// The engine's core with every register stepping on the rising edges of its clock that `step_en` is high on, and
/// every memory reading on those `read_en` is high on. Only the modules under `logos_engine_core` are kept. Each one
/// with a clock, the memories aside, is renamed `<name>_ce`, given `step_en` and `read_en` beside its clock, and has its
/// clocked blocks put under `if (step_en)`; every instance of such a module is renamed and given both enables, and
/// every memory instance becomes a `logos_ram_ce_w<W>` given them. With `step_en` high in the clock's cycle that ends
/// with an engine step and `read_en` high in the cycle after the engine cycle's middle, the core steps as the engine
/// clocked by its own clock does, and a memory reads as its falling-edge read does.
fn enable(verilog: &str) -> String {
    let parts = modules(verilog);
    let is_ram = |n: &str| n.starts_with("logos_ram_w");
    let mut reached: Vec<String> = census(verilog, "logos_engine_core").into_keys().collect();
    reached.push("logos_engine_core".to_string());
    let clocked: Vec<String> =
        parts.iter().filter(|(n, t)| reached.contains(n) && !is_ram(n) && t.contains("(input clk,")).map(|(n, _)| n.clone()).collect();
    let mut widths: Vec<usize> = Vec::new();
    let mut out = String::new();
    for (name, text) in &parts {
        // A memory the enabled memories stand in for, or a module nothing under the core instantiates: only the text
        // before it, the macros it may define.
        if is_ram(name) || (!name.is_empty() && !reached.contains(name)) {
            out.push_str(&text[..text.find(&format!("module {name}")).expect("its header")]);
            continue;
        }
        let mut t = text.clone();
        assert!(!t.contains("step_en") && !t.contains("read_en"), "`{name}` names an enable");
        if clocked.contains(name) {
            let header = t.find(&format!("module {name}")).expect("its header") + "module ".len() + name.len();
            t.insert_str(header, "_ce");
            assert_eq!(t.matches("(input clk, input rst,").count(), 1, "`{name}`'s clock and reset");
            t = t.replacen("(input clk, input rst,", "(input clk, input step_en, input read_en, input rst,", 1);
            let blocks = t.matches("always @(posedge clk) begin").count();
            assert!(blocks >= 1 && t.matches("always @(").count() == blocks, "`{name}`'s every clocked block a rising-edge block");
            t = t.replace("always @(posedge clk) begin", "always @(posedge clk) if (step_en) begin");
        }
        for n in &clocked {
            t = enable_instances(&t, &format!("{n} #("), &format!("{n}_ce #("), ".step_en(step_en), .read_en(read_en), ");
        }
        while let Some(i) = t.find("logos_ram_w") {
            let digits: String = t[i + "logos_ram_w".len()..].chars().take_while(|c| c.is_ascii_digit()).collect();
            let w: usize = digits.parse().expect("a memory's width");
            if !widths.contains(&w) {
                widths.push(w);
            }
            let tail = enable_instances(&t[i..], &format!("logos_ram_w{w} #("), &format!("{} #(", super::ram::ram_ce_name(w)), ".ce(step_en), .cr(read_en), ");
            t = format!("{}{tail}", &t[..i]);
        }
        out.push_str(&t);
        out.push('\n');
    }
    widths.sort();
    format!("{}\n{out}", super::ram::ram_ce_verilog(&widths))
}

/// An engine port as its template declares it: whether it is an input, its width's brackets and its name.
fn engine_port_decls() -> Vec<(bool, String, String)> {
    let (text, _) = engine_ports();
    text.split(',')
        .map(|d| {
            let d = d.trim().trim_start_matches('(').trim();
            let input = d.starts_with("input");
            let name = d.split_whitespace().last().expect("a port's name").to_string();
            let width = d.find('[').map_or(String::new(), |a| d[a..=d.find(']').expect("a width's end")].to_string());
            (input, width, name)
        })
        .collect()
}

/// `logos_engine_bridge`, the crossing between the engine clock the shell runs in (sclk) and the user clock it is
/// divided from (fclk), its every register in the user clock: each value the shell gives the engine taken straight into
/// a register (cap_*) two user cycles after the engine clock's rising edge, so the core, stepping at the next rising
/// edge and reading its memories the cycle after the middle, sees the values the shell set at the edge before, as an
/// engine clocked by the shell's clock does; each value the engine gives the shell taken straight into a register
/// (held_*) two user cycles after the middle, holding past the next rising edge, when the shell takes it.
pub fn bridge_verilog() -> String {
    bridge_of(&engine_port_decls())
}

/// What crosses between the shell and engine `p`: the engine's ports ([`engine_port_decls`]), each context's token-rows
/// side by side for an engine of several ([`Streaming::contexts`]), and the card port's map for a recurrent one
/// ([`bridged_map`]).
fn crossing_ports(p: &EngineParams) -> Vec<(bool, String, String)> {
    let contexts = p.streaming.as_ref().map_or(1, |s| s.contexts);
    let mut ports: Vec<(bool, String, String)> = engine_port_decls()
        .into_iter()
        .map(|(input, width, name)| if name == "rows" && contexts > 1 { (input, format!("[{}:0]", 16 * contexts - 1), name) } else { (input, width, name) })
        .collect();
    ports.extend(bridged_map(p));
    ports
}

/// [`bridge_verilog`] for `crossing`, each `(input, width, name)` as [`engine_port_decls`] gives them.
fn bridge_of(crossing: &[(bool, String, String)]) -> String {
    let mut ports = Vec::new();
    let mut regs = String::new();
    let (mut caps, mut helds, mut assigns) = (String::new(), String::new(), String::new());
    for (input, width, name) in crossing.iter().cloned() {
        if name == "clk" {
            continue;
        }
        let w = if width.is_empty() { String::new() } else { format!("{width} ") };
        if input {
            ports.push(format!("input {w}s_{name}, output {w}f_{name}"));
            writeln!(regs, "  reg {w}cap_{name};").unwrap();
            if name == "rst" {
                writeln!(caps, "    if (frst) cap_rst <= 1'b1; else if (emark2) cap_rst <= s_rst;").unwrap();
            } else {
                writeln!(caps, "    if (emark2) cap_{name} <= s_{name};").unwrap();
            }
            writeln!(assigns, "  assign f_{name} = cap_{name};").unwrap();
        } else {
            ports.push(format!("output {w}s_{name}, input {w}f_{name}"));
            writeln!(regs, "  reg {w}held_{name};").unwrap();
            writeln!(helds, "    if (mark2) held_{name} <= f_{name};").unwrap();
            writeln!(assigns, "  assign s_{name} = held_{name};").unwrap();
        }
    }
    format!(
        r#"// The crossing between the engine clock the shell runs in (sclk) and the user clock it is divided from (fclk), every
// register of it in the user clock (fedge and fmid high the user cycles that end with the engine clock's rising and
// falling edges): each value the shell gives the engine taken straight into a register (cap_*) two user cycles after the
// rising edge, each value the engine gives the shell taken straight into a register (held_*) two user cycles after the
// falling edge. With the engine clock {min} user cycles or more, a path from the shell into a cap_* register has two
// user cycles and one from a held_* register into the shell half an engine cycle less two user cycles.
module logos_engine_bridge
  (input sclk, input fclk, input frst, input fedge, input fmid,
   {ports});
{regs}  reg emark1, emark2, mark1, mark2;
  always @(posedge fclk) begin
    emark1 <= fedge;
    emark2 <= emark1;
    mark1 <= fmid;
    mark2 <= mark1;
{caps}{helds}  end
{assigns}endmodule
"#,
        min = ENGINE_MIN_DIVIDE,
        ports = ports.join(",\n   "),
    )
}

/// `logos_engine`: the shell's engine, its core (`logos_engine_core_ce`) stepping in the user clock (`fclk`) at the
/// engine clock's rising edges (`fedge`), its memories reading the user cycle after its falling edges, the shell's
/// values and the engine's crossing between the engine clock (`clk`) and the user clock through `logos_engine_bridge`;
/// a hybrid engine's matvec jobs on its streaming unit in the user clock (`logos_engine_fast`), with the four channels'
/// read ports.
fn top_verilog(p: &EngineParams) -> String {
    // A streaming unit whose cores are paired runs their processing elements in clk2, twice the fast side's clock.
    let (clk2_in, clk2_pass) = if p.streaming.as_ref().is_some_and(|s| s.set.pe_pairs) { (" input clk2,", ".clk2(clk2), ") } else { ("", "") };
    if let Some(s) = p.streaming.as_ref().filter(|s| s.contexts > 1) {
        return top_contexts_verilog(p, s);
    }
    let decls = engine_port_decls();
    let (ports, _) = engine_ports();
    let mut wires = String::new();
    let mut bridge = vec![".sclk(clk), .fclk(fclk), .frst(frst), .fedge(fedge), .fmid(fmid)".to_string()];
    let mut core = vec![".clk(fclk), .step_en(fedge), .read_en(cr)".to_string()];
    for (_, width, name) in &decls {
        if name == "clk" {
            continue;
        }
        let w = if width.is_empty() { String::new() } else { format!("{width} ") };
        writeln!(wires, "  wire {w}f_{name};").unwrap();
        bridge.push(format!(".s_{name}({name}), .f_{name}(f_{name})"));
        core.push(format!(".{name}(f_{name})"));
    }
    for (_, width, name) in &bridged_map(p) {
        let w = if width.is_empty() { String::new() } else { format!("{width} ") };
        writeln!(wires, "  wire {w}f_{name};").unwrap();
        bridge.push(format!(".s_{name}({name}), .f_{name}(f_{name})"));
    }
    let (fast_ports, fast) = match &p.streaming {
        None => (String::new(), String::new()),
        Some(s) => {
            use super::stream::{AW, LW};
            let sa = store_address_bits(p);
            let (j, mdt, drt) = (HybridJob::BITS, 4 * (AW + LW + 1) - 1, 4 * 8 * s.layout.line_bytes - 1);
            core.push(".sm_own(own), .sm_go(go), .sm_job(job), .sm_busy(held_busy), .e_ra(e_ra), .e_we(e_we), .e_wa(e_wa), .e_wd(e_wd), .e_wm(e_wm), .e_rd(e_rd)".to_string());
            if s.vectors {
                core.push(".sm_vec_go(vec_go), .sm_vec_job(vec_job), .sm_vec_fault(vec_fault)".to_string());
            }
            if s.attention {
                core.push(".sm_att_go(att_go), .sm_att_job(att_job)".to_string());
            }
            let (attention_wires, attention) = if s.attention {
                (format!("  wire att_go;\n  wire [{}:0] att_job;\n", AttendJob::BITS - 1), ",\n    .att_go(att_go), .att_job(att_job)".to_string())
            } else {
                (String::new(), String::new())
            };
            let (vector_wires, vectors) = if s.vectors {
                (
                    format!("  wire vec_go, vec_fault;\n  wire [{}:0] vec_job;\n", VectorSideJob::BITS - 1),
                    ",\n    .vec_go(vec_go), .vec_job(vec_job), .vec_fault(vec_fault)"
                        .to_string(),
                )
            } else {
                (String::new(), String::new())
            };
            let (recurrence_ports, recurrence_wires, recurrence) = if s.recurrence {
                core.push(".sm_gdn_go(gdn_go), .sm_gdn_job(gdn_job), .sm_gdn_fault(gdn_fault)".to_string());
                (
                    format!(
                        ",\n   output [3:0] sm_mw_valid, input [3:0] sm_mw_ready, output [{drt}:0] sm_mw_data,\n   \
                         input [3:0] sm_mdone_valid, output [3:0] sm_mdone_ready,\n   \
                         input [31:0] map_space, input [{aw}:0] map_base, input [4:0] map_granule_log2",
                        aw = AW - 1
                    ),
                    format!(
                        "  wire gdn_go, gdn_fault;\n  wire [{}:0] gdn_job;\n  wire [3:0] p_mw_valid, p_mw_ready, p_mdone_valid, p_mdone_ready;\n  \
                         wire [{drt}:0] p_mw_data;\n",
                        GdnJob::bits(s) - 1
                    ),
                    ",\n    .mw_valid(p_mw_valid), .mw_ready(p_mw_ready), .mw_data(p_mw_data), .mdone_valid(p_mdone_valid),\n    \
                     .mdone_ready(p_mdone_ready), .gdn_go(gdn_go), .gdn_job(gdn_job), .gdn_fault(gdn_fault),\n    \
                     .host_wr(f_host_wr), .host_table(f_host_table), .host_addr(f_host_addr), .host_wdata(f_host_wdata),\n    \
                     .map_space(f_map_space), .map_base(f_map_base), .map_granule_log2(f_map_granule_log2)"
                        .to_string(),
                )
            } else {
                (String::new(), String::new(), String::new())
            };
            (
                format!(
                    ",\n   output [3:0] sm_md_valid, input [3:0] sm_md_ready, output [{mdt}:0] sm_md_data,\n   input [3:0] sm_mr_valid, output [3:0] sm_mr_ready, input [{drt}:0] sm_mr_data{recurrence_ports}"
                ),
                format!(
                    r#"  wire own, go, held_busy, e_we;
  wire [{jt}:0] job;
  wire [31:0] e_ra, e_wa;
  wire [511:0] e_wd, e_rd;
  wire [63:0] e_wm;
{recurrence_wires}{vector_wires}{attention_wires}{pipes}  logos_engine_fast fast(.clk(fclk), {clk2_pass}.rst(frst), .ce(fedge), .cr(cr), .own(own), .go(go), .job(job), .e_ra(e_ra[{st}:0]),
    .e_we(e_we), .e_wa(e_wa[{st}:0]), .e_wd(e_wd), .e_wm(e_wm), .rd(e_rd), .busy_held(held_busy), .md_valid(p_md_valid),
    .md_ready(p_md_ready), .md_data(p_md_data), .mr_valid(p_mr_valid), .mr_ready(p_mr_ready), .mr_data(p_mr_data){recurrence}{vectors}{attention});
"#,
                    pipes = channel_pipes(s),
                    jt = j - 1,
                    st = sa - 1,
                ),
            )
        }
    };
    format!(
        r#"// The engine: its core stepping in the user clock (fclk) at the engine clock's rising edges (fedge) and reading its
// memories the user cycle after the falling edges (cr), the shell's values and its own through the crossing between the
// engine clock (clk) and the user clock{hybrid}.
module logos_engine
{top_ports},
   input fclk,{clk2_in} input frst, input fedge, input fmid{fast_ports});
  // The cycle after the engine clock's falling edge, the core's memories' read: a register its loads are spread from.
  (* maxfan = 2000 *) reg cr;
  always @(posedge fclk) cr <= fmid;
{wires}  logos_engine_bridge bridge({bridge});
{fast}  logos_engine_core_ce core({core});
endmodule
"#,
        hybrid = match &p.streaming {
            Some(s) if s.vectors => "; its matvec jobs, its lazily flushed memories' token-rows and its vector jobs on the units in the user clock",
            Some(s) if s.recurrence => "; its matvec jobs and its lazily flushed memories' token-rows on the units in the user clock",
            Some(_) => "; its matvec jobs on the streaming unit in the user clock",
            None => "",
        },
        top_ports = ports.replace("output reg ", "output "),
        bridge = bridge.join(",\n    "),
        core = core.join(",\n    "),
    )
}

/// `logos_engine` for an engine of [`Streaming::contexts`] contexts ([`fast_contexts`]): a core a context, each
/// started with the step when its context has token-rows, its rows from its own field of `rows`; each takes the host's
/// writes to the tables every context shares and those to its own rows table and bank of the store — its context in the
/// host address's top four bits, cleared in the core's — and the host reads the bank `host_raddr`'s top bits name. The
/// engine is busy while any core is, faults when any does, and gives the shell each core's program counter in 16 bits,
/// context 0's lowest. The cores read the card through its one port a read at a time, the first asking after the last
/// served, each answer going to the core that asked; no core of such an engine writes the card.
fn top_contexts_verilog(p: &EngineParams, s: &Streaming) -> String {
    // A streaming unit whose cores are paired runs their processing elements in clk2, twice the fast side's clock.
    let (clk2_in, clk2_pass) = if s.set.pe_pairs { (" input clk2,", ".clk2(clk2), ") } else { ("", "") };
    let c = s.contexts;
    let cb = if c > 1 { (usize::BITS - (c - 1).leading_zeros()) as usize } else { 1 };
    let sa = store_address_bits(p);
    let (j, g, v, a) = (HybridJob::BITS, GdnJob::bits(s), VectorSideJob::BITS, AttendJob::BITS);
    let drt = 4 * 8 * s.layout.line_bytes as usize - 1;
    let (ports, _) = engine_ports();
    assert_eq!(ports.matches("input [15:0] rows,").count(), 1, "the engine's rows port");
    let top_ports = ports.replace("input [15:0] rows,", &format!("input [{}:0] rows,", 16 * c - 1)).replace("output reg ", "output ");
    let mut wires = String::new();
    let mut bridge = vec![".sclk(clk), .fclk(fclk), .frst(frst), .fedge(fedge), .fmid(fmid)".to_string()];
    for (_, width, name) in &crossing_ports(p) {
        if name == "clk" {
            continue;
        }
        let w = if width.is_empty() { String::new() } else { format!("{width} ") };
        writeln!(wires, "  wire {w}f_{name};").unwrap();
        bridge.push(format!(".s_{name}({name}), .f_{name}(f_{name})"));
    }
    let field = |bits: usize, k: usize| format!("[{}:{}]", bits * k + bits - 1, bits * k);
    let mut cores = String::new();
    for k in 0..c {
        let mut conns = vec![".clk(fclk), .step_en(fedge), .read_en(cr)".to_string()];
        for (_, _, name) in engine_port_decls() {
            if name == "clk" {
                continue;
            }
            let to = match name.as_str() {
                "start" => format!("f_start && f_rows{} != 16'd0", field(16, k)),
                "rows" => format!("f_rows{}", field(16, k)),
                "busy" => format!("c_busy[{k}]"),
                "fault" => format!("c_fault[{k}]"),
                "pc" => format!("c_pc{}", field(32, k)),
                "host_wr" => format!("c_host_wr[{k}]"),
                "host_addr" => "c_host_addr".to_string(),
                "host_raddr" => "c_host_raddr".to_string(),
                "host_rdata" => format!("c_host_rdata{}", field(512, k)),
                "mem_rd" => format!("c_mem_rd[{k}]"),
                "mem_rd_line" => format!("c_mem_rd_line{}", field(32, k)),
                "mem_rd_valid" => format!("c_mem_rd_valid[{k}]"),
                "mem_wr" => format!("c_mem_wr[{k}]"),
                "mem_wr_line" => format!("c_mem_wr_line{}", field(32, k)),
                "mem_wr_data" => format!("c_mem_wr_data{}", field(512, k)),
                "mem_wr_mask" => format!("c_mem_wr_mask{}", field(64, k)),
                "mem_wr_ready" => "1'b0".to_string(),
                _ => format!("f_{name}"),
            };
            conns.push(format!(".{name}({to})"));
        }
        conns.push(format!(
            ".sm_own(own[{k}]), .sm_go(go[{k}]), .sm_job(job{jf}), .sm_busy(held_busy[{k}]), .e_ra(e_ra{e32}), .e_we(e_we[{k}]),\n    \
             .e_wa(e_wa{e32}), .e_wd(e_wd{e512}), .e_wm(e_wm{e64}), .e_rd(e_rd{e512})",
            jf = field(j, k),
            e32 = field(32, k),
            e512 = field(512, k),
            e64 = field(64, k),
        ));
        conns.push(format!(".sm_vec_go(vec_go[{k}]), .sm_vec_job(vec_job{}), .sm_vec_fault(vec_fault[{k}])", field(v, k)));
        conns.push(format!(".sm_att_go(att_go[{k}]), .sm_att_job(att_job{})", field(a, k)));
        conns.push(format!(".sm_gdn_go(gdn_go[{k}]), .sm_gdn_job(gdn_job{}), .sm_gdn_fault(gdn_fault[{k}])", field(g, k)));
        writeln!(cores, "  logos_engine_core_ce core{k}({});", conns.join(",\n    ")).unwrap();
        writeln!(cores, "  assign c_host_wr[{k}] = f_host_wr && (host_shared || f_host_addr[31:28] == 4'd{k});").unwrap();
    }
    let packed = |name: &str| (0..c).rev().map(|k| format!("{name}[{}:{}]", 32 * k + sa - 1, 32 * k)).collect::<Vec<_>>().join(", ");
    let pcs = (0..c).rev().map(|k| format!("c_pc[{}:{}]", 32 * k + 15, 32 * k)).collect::<Vec<_>>().join(", ");
    format!(
        r#"// The engine of {c} contexts: a core a context stepping in the user clock (fclk) at the engine clock's rising edges
// (fedge) and reading its memories the user cycle after the falling edges (cr), the shell's values and the engine's
// through the crossing between the engine clock (clk) and the user clock; the units in the user clock, each class of
// them serving one context at a time.
module logos_engine
{top_ports},
   input fclk,{clk2_in} input frst, input fedge, input fmid,
   output [3:0] sm_md_valid, input [3:0] sm_md_ready, output [{mdt}:0] sm_md_data,
   input [3:0] sm_mr_valid, output [3:0] sm_mr_ready, input [{drt}:0] sm_mr_data,
   output [3:0] sm_mw_valid, input [3:0] sm_mw_ready, output [{drt}:0] sm_mw_data,
   input [3:0] sm_mdone_valid, output [3:0] sm_mdone_ready,
   input [31:0] map_space, input [{awt}:0] map_base, input [4:0] map_granule_log2);
  // The cycle after the engine clock's falling edge, the core's memories' read: a register its loads are spread from.
  (* maxfan = 2000 *) reg cr;
  always @(posedge fclk) cr <= fmid;
{wires}  logos_engine_bridge bridge({bridge});
  wire [{ct}:0] own, go, held_busy, e_we;
  wire [{cj}:0] job;
  wire [{c32}:0] e_ra, e_wa;
  wire [{c512}:0] e_wd, e_rd;
  wire [{c64}:0] e_wm;
  wire [{ct}:0] gdn_go, gdn_fault;
  wire [{cg}:0] gdn_job;
  wire [3:0] p_mw_valid, p_mw_ready, p_mdone_valid, p_mdone_ready;
  wire [{drt}:0] p_mw_data;
  wire [{ct}:0] vec_go, vec_fault;
  wire [{cv}:0] vec_job;
  wire [{ct}:0] att_go;
  wire [{ca}:0] att_job;
{pipes}  logos_engine_fast fast(.clk(fclk), {clk2_pass}.rst(frst), .ce(fedge), .cr(cr), .own(own), .go(go), .job(job), .e_ra({{{e_ra}}}),
    .e_we(e_we), .e_wa({{{e_wa}}}), .e_wd(e_wd), .e_wm(e_wm), .rd(e_rd), .busy_held(held_busy), .md_valid(p_md_valid),
    .md_ready(p_md_ready), .md_data(p_md_data), .mr_valid(p_mr_valid), .mr_ready(p_mr_ready), .mr_data(p_mr_data),
    .mw_valid(p_mw_valid), .mw_ready(p_mw_ready), .mw_data(p_mw_data), .mdone_valid(p_mdone_valid),
    .mdone_ready(p_mdone_ready), .gdn_go(gdn_go), .gdn_job(gdn_job), .gdn_fault(gdn_fault), .map_space(f_map_space),
    .map_base(f_map_base), .map_granule_log2(f_map_granule_log2),
    .vec_go(vec_go), .vec_job(vec_job), .vec_fault(vec_fault), .host_wr(f_host_wr), .host_table(f_host_table),
    .host_addr(f_host_addr), .host_wdata(f_host_wdata),
    .att_go(att_go), .att_job(att_job));
  // Each core takes the host's writes to the tables every context shares, and those to its own rows table and bank
  // of the store, the context in the host address's top four bits, cleared in the core's.
  wire host_shared = f_host_table != 3'd1 && f_host_table < 3'd6;
  wire [{ct}:0] c_host_wr, c_busy, c_fault, c_mem_rd, c_mem_wr, c_mem_rd_valid;
  wire [31:0] c_host_addr = {{4'd0, f_host_addr[27:0]}};
  wire [31:0] c_host_raddr = {{4'd0, f_host_raddr[27:0]}};
  wire [{c512}:0] c_host_rdata, c_mem_wr_data;
  wire [{c32}:0] c_pc, c_mem_rd_line, c_mem_wr_line;
  wire [{c64}:0] c_mem_wr_mask;
{cores}  assign f_host_rdata = c_host_rdata[512*f_host_raddr[{cbh}:28] +: 512];
  assign f_busy = |c_busy;
  assign f_fault = |c_fault;
  assign f_pc = {{{pcs}}};
  // The cores' reads of the card through its one port, a read at a time.
  function [{cbt}:0] after(input [{ct}:0] want, input [{cbt}:0] last);
    integer i, k;
    reg found;
    begin
      found = 1'b0;
      after = last;
      for (i = 1; i <= {c}; i = i + 1) begin
        k = (last + i) % {c};
        if (!found && want[k]) begin found = 1'b1; after = k[{cbt}:0]; end
      end
    end
  endfunction
  reg [{ct}:0] rd_pend;
  reg [{c32}:0] rd_pend_line;
  reg rd_out;
  reg [{cbt}:0] rd_owner;
  wire [{ct}:0] rd_want = rd_pend | c_mem_rd;
  wire [{cbt}:0] rd_to = after(rd_want, rd_owner);
  wire rd_issue = rd_want != {c}'d0 && (!rd_out || f_mem_rd_valid);
  assign f_mem_rd = rd_issue;
  assign f_mem_rd_line = rd_pend[rd_to] ? rd_pend_line[32*rd_to +: 32] : c_mem_rd_line[32*rd_to +: 32];
  genvar rd_at;
  generate for (rd_at = 0; rd_at < {c}; rd_at = rd_at + 1) begin : rd_route
    assign c_mem_rd_valid[rd_at] = rd_out && f_mem_rd_valid && rd_owner == rd_at;
  end endgenerate
  assign f_mem_wr = 1'b0;
  assign f_mem_wr_line = 32'd0;
  assign f_mem_wr_data = 512'd0;
  assign f_mem_wr_mask = 64'd0;
  integer rq;
  always @(posedge fclk) if (fedge) begin
    if (f_rst) begin rd_pend <= {c}'d0; rd_out <= 1'b0; rd_owner <= {cb}'d0; end
    else begin
      for (rq = 0; rq < {c}; rq = rq + 1)
        if (c_mem_rd[rq] && !(rd_issue && rd_to == rq[{cbt}:0])) rd_pend_line[32*rq +: 32] <= c_mem_rd_line[32*rq +: 32];
      rd_pend <= rd_want & ~({{{{{cm}{{1'b0}}}}, rd_issue}} << rd_to);
      if (rd_issue) begin rd_out <= 1'b1; rd_owner <= rd_to; end
      else if (f_mem_rd_valid) rd_out <= 1'b0;
    end
  end
endmodule
"#,
        mdt = 4 * (super::stream::AW as usize + super::stream::LW as usize + 1) - 1,
        awt = super::stream::AW - 1,
        ct = c - 1,
        cm = c - 1,
        cbt = cb - 1,
        cbh = 28 + cb - 1,
        cj = c * j - 1,
        c32 = 32 * c - 1,
        c512 = 512 * c - 1,
        c64 = 64 * c - 1,
        cg = c * g - 1,
        cv = c * v - 1,
        ca = c * a - 1,
        e_ra = packed("e_ra"),
        e_wa = packed("e_wa"),
        pipes = channel_pipes(s),
        bridge = bridge.join(",\n    "),
    )
}

/// The pipes each channel's fast port crosses at the engine's edge, each way: a path from a channel's bank at the die's
/// edge to the fast side a pipe each part of the way.
pub const PORT_PIPES: usize = 3;

/// Each channel's fast port through [`PORT_PIPES`] pipes each way at the engine's edge ([`top_verilog`]'s): the
/// descriptors and lines, and a recurrent engine's writes and write-done tokens.
fn channel_pipes(s: &Streaming) -> String {
    use super::stream::{AW, LW};
    let (mdt, drt) = (4 * (AW + LW + 1) - 1, 4 * 8 * s.layout.line_bytes - 1);
    format!(
        r#"  // Each channel's fast port through PORT_PIPES pipes each way at the engine's edge, so the path from the channel's bank
  // to the fast side crosses the die a pipe each part of the way.
  wire [3:0] p_md_valid, p_md_ready, p_mr_valid, p_mr_ready;
  wire [{mdt}:0] p_md_data;
  wire [{drt}:0] p_mr_data;
  genvar pipe_ch;
  generate for (pipe_ch = 0; pipe_ch < 4; pipe_ch = pipe_ch + 1) begin : port_pipe
    logos_pipe_chain #(.W({mdw}), .K({k})) md(.clk(fclk), .rst_n(!frst), .in_valid(p_md_valid[pipe_ch]), .in_ready(p_md_ready[pipe_ch]),
      .in_data(p_md_data[{mdw}*pipe_ch +: {mdw}]), .out_valid(sm_md_valid[pipe_ch]), .out_ready(sm_md_ready[pipe_ch]),
      .out_data(sm_md_data[{mdw}*pipe_ch +: {mdw}]));
    logos_pipe_chain #(.W({dw}), .K({k})) mr(.clk(fclk), .rst_n(!frst), .in_valid(sm_mr_valid[pipe_ch]), .in_ready(sm_mr_ready[pipe_ch]),
      .in_data(sm_mr_data[{dw}*pipe_ch +: {dw}]), .out_valid(p_mr_valid[pipe_ch]), .out_ready(p_mr_ready[pipe_ch]),
      .out_data(p_mr_data[{dw}*pipe_ch +: {dw}]));
{write_pipes}  end endgenerate
"#,
        mdw = AW + LW + 1,
        dw = 8 * s.layout.line_bytes,
        k = PORT_PIPES,
        write_pipes = if s.recurrence {
            format!(
                "    logos_pipe_chain #(.W({dw}), .K({k})) mw(.clk(fclk), .rst_n(!frst), .in_valid(p_mw_valid[pipe_ch]), .in_ready(p_mw_ready[pipe_ch]),\n      \
                 .in_data(p_mw_data[{dw}*pipe_ch +: {dw}]), .out_valid(sm_mw_valid[pipe_ch]), .out_ready(sm_mw_ready[pipe_ch]),\n      \
                 .out_data(sm_mw_data[{dw}*pipe_ch +: {dw}]));\n    wire done_bit;\n    \
                 logos_pipe_chain #(.W(1), .K({k})) mdone(.clk(fclk), .rst_n(!frst), .in_valid(sm_mdone_valid[pipe_ch]), .in_ready(sm_mdone_ready[pipe_ch]),\n      \
                 .in_data(1'b0), .out_valid(p_mdone_valid[pipe_ch]), .out_ready(p_mdone_ready[pipe_ch]), .out_data(done_bit));\n",
                dw = 8 * s.layout.line_bytes,
                k = PORT_PIPES
            )
        } else {
            String::new()
        },
    )
}

/// Bits of a store beat's address.
fn store_address_bits(p: &EngineParams) -> usize {
    if p.store_beats > 1 {
        (usize::BITS - (p.store_beats - 1).leading_zeros()) as usize
    } else {
        1
    }
}

/// The engine's ports as its template declares them, and their names in order.
fn engine_ports() -> (&'static str, Vec<&'static str>) {
    let start = ENGINE_VERILOG.find("  (input clk").expect("the engine's ports");
    let end = ENGINE_VERILOG.find("@PORTS@);").expect("the engine's ports' end");
    let text = &ENGINE_VERILOG[start..end];
    let names = text.split(',').map(|d| d.split_whitespace().last().expect("a port's name")).collect();
    (text, names)
}


/// `logos_engine_fast`, a hybrid engine's user-clock side: its streaming unit and the token-row store, the store the
/// unit's while the core gives it the job (`own`) and the core's otherwise, read the cycle after the engine cycle's
/// middle (`cr`) and written at its step (`ce`) as the core's own memories are; the unit started the cycle after the
/// middle of the engine cycle the core gives it the job in (`go`), its busy held from the cycle after each middle.
fn fast_verilog(p: &EngineParams, s: &Streaming) -> String {
    // A streaming unit whose cores are paired runs their processing elements in clk2, twice the fast side's clock.
    let (clk2_in, clk2_pass) = if s.set.pe_pairs { (" input clk2,", ".clk2(clk2), ") } else { ("", "") };
    use super::stream::{AW, LW};
    let sa = store_address_bits(p);
    let j = HybridJob::BITS;
    let set = &s.set;
    // A token-row's activation words: a row's blocks of the format with the most, an octet's eight blocks one word.
    let nb_max = p.formats.iter().map(|&(ty, blocks)| (blocks * ty.block_len()).div_ceil(set.block_len(ty))).max().unwrap_or(1);
    let mdw = AW + LW + 1;
    let dw = 8 * s.layout.line_bytes;
    let text = format!(
        r#"// A hybrid engine's user-clock side: the streaming unit and the token-row store, the store the unit's while it owns it
// (own), the core's otherwise, read the cycle after the engine cycle's middle (cr) and written at its step (ce).
module logos_engine_fast
  (input clk,{clk2_in} input rst, input ce, input cr, input own, input go, input [{jt}:0] job,
   input [{st}:0] e_ra, input e_we, input [{st}:0] e_wa, input [511:0] e_wd, input [63:0] e_wm,
   output [511:0] rd, output reg busy_held,
   output [3:0] md_valid, input [3:0] md_ready, output [{mdt}:0] md_data,
   input [3:0] mr_valid, output [3:0] mr_ready, input [{drt}:0] mr_data);
  wire busy, u_we;
  wire start = go && cr;
  wire [{st}:0] u_ra, u_wa;
  wire [511:0] u_wd;
  wire [63:0] u_wm;
  wire [1023:0] runs = job[{runs_top}:{runs}];
  logos_streaming_matvec_{name} #(.N({n}), .T_MAX({t_max}), .NB_MAX({nb_max}), .SA({sa})) unit(.clk(clk), {clk2_pass}.rst(rst), .start(start),
    .greedy(job[{greedy}]), {fold}{fmt}.blocks(job[47:32]), .tokens(job[{tokens_top}:{tokens}]), {two}.rows(job[95:64]), .stripes(job[111:96]),
    .turns(job[135:128]), .from_base(job[{from_base}]), .from_beats(job[{from_beats}]), .to_base(job[{to_base}]),
    .to_beats(job[{to_beats}]), .busy(busy), .st_ra(u_ra), .st_rd(rd), .st_we(u_we), .st_wa(u_wa), .st_wd(u_wd),
    .st_wm(u_wm), .tr_count({count}), .tr_line({line}), .tr_lines({lines}), .tr_bytes({bytes}),
    .tr_last_lines({last_lines}), .tr_last_bytes({last_bytes}), .tr_row({row}), .tr_row_step({row_step}),
    .tensor(job[{tensor_top}:160]), .md_valid(md_valid), .md_ready(md_ready), .md_data(md_data), .mr_valid(mr_valid),
    .mr_ready(mr_ready), .mr_data(mr_data));
  always @(posedge clk)
    if (rst) busy_held <= 1'b0;
    else if (cr) busy_held <= busy || start;
  logos_bram_re_w512 #(.D({store}), .A({sa})) store(.clk(clk), .re(own || cr), .ra(own ? u_ra : e_ra), .rd(rd),
    .we(own ? u_we : ce && e_we), .wa(own ? u_wa : e_wa), .wd(own ? u_wd : e_wd), .wm(own ? u_wm : e_wm));
endmodule
"#,
        jt = j - 1,
        st = sa - 1,
        mdt = 4 * mdw - 1,
        drt = 4 * dw - 1,
        runs = HybridJob::RUNS,
        runs_top = HybridJob::RUNS + 1023,
        greedy = HybridJob::GREEDY,
        fold = if s.folds > 0 { format!(".fold(job[{}:{}]), ", HybridJob::FOLD + 1, HybridJob::FOLD) } else { String::new() },
        name = set.name,
        n = s.pairs(),
        t_max = p.rows,
        fmt = if set.formats.len() > 1 { ".fmt(job[3:0]), " } else { "" },
        two = if set.words == Words::Two { format!(".two(job[{}]), ", HybridJob::TWO) } else { String::new() },
        tokens = HybridJob::TOKENS,
        tokens_top = HybridJob::TOKENS + 7,
        from_base = format!("{}:{}", HybridJob::FROM + sa - 1, HybridJob::FROM),
        from_beats = format!("{}:{}", HybridJob::FROM + 47, HybridJob::FROM + 32),
        to_base = format!("{}:{}", HybridJob::TO + sa - 1, HybridJob::TO),
        to_beats = format!("{}:{}", HybridJob::TO + 47, HybridJob::TO + 32),
        count = runs_field(0, 32),
        line = runs_field(1, AW),
        lines = runs_field(2, LW),
        bytes = runs_field(3, super::stream::BYW),
        last_lines = runs_field(4, LW),
        last_bytes = runs_field(5, super::stream::BYW),
        row = runs_field(6, super::stream::ROWW),
        row_step = runs_field(7, super::stream::ROWW),
        tensor_top = 160 + super::stream::TW - 1,
        store = p.store_beats,
    );
    let text = if s.recurrence { fast_recurrence(&text, p, s) } else { text };
    let text = if s.vectors { fast_vectors(&text, p) } else { text };
    let text = if s.attention { fast_attention(&text, p) } else { text };
    let text = if s.recurrence_lanes > 1 { fast_lanes(&text, p, s) } else { text };
    if s.contexts > 1 {
        fast_contexts(&text, p, s)
    } else {
        text
    }
}

/// `logos_engine_fast` with its recurrence unit on head lanes ([`Streaming::recurrence_lanes`]), from the text of every
/// other unit beside it: `logos_gdn_lanes` for `logos_gdn_side`, each lane's rows through a `logos_fast_port` of its
/// own, the shared port (the coordinator's and the vector and attention sides') and the lanes' onto the channels
/// through `logos_channel_mux`, the shared port the mux's port 0 and lane k's port 1 + k; the recurrence's fault any
/// port's or the lanes' own.
fn fast_lanes(text: &str, p: &EngineParams, s: &Streaming) -> String {
    use super::stream::{AW, LW};
    let (hd, flush) = p.delta.expect("a recurrence unit runs a model's Gated-DeltaNet blocks");
    let (t_max, window) = recurrence_log(flush);
    let sa = store_address_bits(p);
    let (l, dw, mdw) = (s.recurrence_lanes, 8 * s.layout.line_bytes as usize, (AW + LW + 1) as usize);
    let lba = (usize::BITS - (s.log_lines.max(2) - 1).leading_zeros()) as usize;
    let ports = 4 * (l + 1);
    let lanes = format!(
        r#"  // The recurrence's head lanes: each lane's rows and flushed rows through a fast port of its own, the lanes' ports and
  // the shared one onto the channels at once (port 0 the shared one, port 1 + k lane k's).
  wire [{lt}:0] l_rq_valid, l_rq_ready, l_rs_valid, l_wq_valid, l_wq_ready, l_writing, l_fault;
  wire [{lt32}:0] l_rq_line, l_wq_line;
  wire [{ltd}:0] l_rs_data, l_wq_data;
  wire p_fault, lanes_fault;
  wire [{pt}:0] c_md_valid, c_md_ready, c_mr_valid, c_mr_ready, c_mw_valid, c_mw_ready, c_mdone_valid, c_mdone_ready;
  wire [{pmd}:0] c_md_data;
  wire [{pdt}:0] c_mw_data;
  genvar lk;
  generate
    for (lk = 0; lk < {l}; lk = lk + 1) begin : lane_port
      logos_fast_port #(.NB(4), .AW({aw}), .LW({lw}), .DL({dw})) port(.clk(clk), .rst(rst), .space(map_space), .base(map_base),
        .granule_log2(map_granule_log2), .clear(g_start), .fault(l_fault[lk]), .rq_valid(l_rq_valid[lk]),
        .rq_ready(l_rq_ready[lk]), .rq_line(l_rq_line[32*lk +: 32]), .rs_valid(l_rs_valid[lk]),
        .rs_data(l_rs_data[{dw}*lk +: {dw}]), .wq_valid(l_wq_valid[lk]), .wq_ready(l_wq_ready[lk]),
        .wq_line(l_wq_line[32*lk +: 32]), .wq_data(l_wq_data[{dw}*lk +: {dw}]), .writing(l_writing[lk]),
        .md_valid(c_md_valid[4*(lk+1) +: 4]), .md_ready(c_md_ready[4*(lk+1) +: 4]),
        .md_data(c_md_data[{mdw4}*(lk+1) +: {mdw4}]), .mr_valid(c_mr_valid[4*(lk+1) +: 4]),
        .mr_ready(c_mr_ready[4*(lk+1) +: 4]), .mr_data(mr_data), .mw_valid(c_mw_valid[4*(lk+1) +: 4]),
        .mw_ready(c_mw_ready[4*(lk+1) +: 4]), .mw_data(c_mw_data[{dw4}*(lk+1) +: {dw4}]),
        .mdone_valid(c_mdone_valid[4*(lk+1) +: 4]), .mdone_ready(c_mdone_ready[4*(lk+1) +: 4]));
    end
  endgenerate
  // The ports straight onto the multiplexer, which holds each port's descriptors and write lines in registers of its own.
  logos_channel_mux #(.P({np}), .NB(4), .AW({aw}), .LW({lw}), .DL({dw})) mux(.clk(clk), .rst(rst), .pd_valid(c_md_valid),
    .pd_ready(c_md_ready), .pd_data(c_md_data), .pr_valid(c_mr_valid), .pr_ready(c_mr_ready), .pw_valid(c_mw_valid),
    .pw_ready(c_mw_ready), .pw_data(c_mw_data), .pdone_valid(c_mdone_valid), .pdone_ready(c_mdone_ready),
    .md_valid(p_md_valid), .md_ready(md_ready), .md_data(p_md_data), .mr_valid(mr_valid), .mr_ready(p_mr_ready),
    .mw_valid(mw_valid), .mw_ready(mw_ready), .mw_data(mw_data), .mdone_valid(mdone_valid), .mdone_ready(mdone_ready));
  assign gdn_fault = p_fault || l_fault != {l}'d0 || lanes_fault;
"#,
        lt = l - 1,
        lt32 = 32 * l - 1,
        ltd = dw * l - 1,
        pt = ports - 1,
        pmd = ports * mdw - 1,
        pdt = ports * dw - 1,
        mdw4 = 4 * mdw,
        dw4 = 4 * dw,
        np = l + 1,
        aw = AW,
        lw = LW,
    );
    let w = s.recurrence_width;
    let fl = s.recurrence_flushes;
    assert!(
        hd % (fl * w * FADD_LATENCY) == 0,
        "a {hd}-wide head's rows split over {fl} flushes, each a whole number of the flush's groups of {} rows",
        w * FADD_LATENCY
    );
    if let Some(window) = window {
        assert!(fl == 1, "a trailing flush replays its shares on one flush unit a lane, not {fl}");
        assert!(
            hd % window == 0 && (hd / window) % (w * FADD_LATENCY) == 0,
            "a trailing flush of {window} tokens replays a {hd}-wide head's rows in shares of a whole number of the read-out's groups of {} rows",
            w * FADD_LATENCY
        );
    }
    let text = replaced(
        text,
        &[
            (
                &format!("  logos_gdn_side #(.HD({hd}), .TMAX({t_max}), .SA({sa}), .W({w})) side("),
                &format!(
                    "{lanes}  logos_gdn_lanes #(.HD({hd}), .TMAX({t_max}), .SA({sa}), {credit}.W({w}), .L({l}), .LBA({lba}), .LOG_LINES({ll}){flushes}) side(",
                    ll = s.log_lines,
                    credit = if s.recurrence_credit != super::gdn::LANE_CREDIT { format!(".OUTSTANDING({}), ", s.recurrence_credit) } else { String::new() },
                    flushes = if fl > 1 { format!(", .FLUSHES({fl})") } else { String::new() }
                ),
            ),
            (
                "\n    .written(!writing));\n",
                &format!(
                    "\n    .written(!writing), .fault(lanes_fault), .l_rq_valid(l_rq_valid), .l_rq_ready(l_rq_ready),\n    \
                     .l_rq_line(l_rq_line), .l_rs_valid(l_rs_valid), .l_rs_data(l_rs_data), .l_wq_valid(l_wq_valid),\n    \
                     .l_wq_ready(l_wq_ready), .l_wq_line(l_wq_line), .l_wq_data(l_wq_data), .l_written(~l_writing),\n    \
                     .rows(gdn_job[{}:{}]), .row_valid(g_row), .row_ready(g_take));\n",
                    GdnJob::ROWS + 15,
                    GdnJob::ROWS
                ),
            ),
            (".fault(gdn_fault), .rq_valid(", ".fault(p_fault), .rq_valid("),
            (
                "  wire g_row, g_take, g_side_busy;\n",
                "  wire g_row, g_take;\n",
            ),
            (
                "  reg g_took;\n  always @(posedge clk) g_took <= !rst && g_take;\n  assign g_take = g_row && !g_side_busy && !g_took && !writing;\n  assign g_busy = g_side_busy || g_row;\n",
                "",
            ),
            (".start(g_take), .busy(g_side_busy),", ".start(g_start), .busy(g_busy),"),
            (
                ".md_valid(p_md_valid), .md_ready(md_ready), .md_data(p_md_data),\n    \
                 .mr_valid(mr_valid), .mr_ready(p_mr_ready), .mr_data(mr_data), .mw_valid(mw_valid), .mw_ready(mw_ready),\n    \
                 .mw_data(mw_data), .mdone_valid(mdone_valid), .mdone_ready(mdone_ready));\n",
                &format!(
                    ".md_valid(c_md_valid[3:0]), .md_ready(c_md_ready[3:0]),\n    \
                     .md_data(c_md_data[{m}:0]), .mr_valid(c_mr_valid[3:0]), .mr_ready(c_mr_ready[3:0]), .mr_data(mr_data),\n    \
                     .mw_valid(c_mw_valid[3:0]), .mw_ready(c_mw_ready[3:0]), .mw_data(c_mw_data[{d}:0]), .mdone_valid(c_mdone_valid[3:0]),\n    \
                     .mdone_ready(c_mdone_ready[3:0]));\n",
                    m = 4 * mdw - 1,
                    d = 4 * dw - 1
                ),
            ),
        ],
    );
    if !s.delta_norm {
        return text;
    }
    // The job's gated norm: each row's gate beat from the rows' slots, the norm's fields from the job word, to the lanes.
    let field = |at: usize, bits: usize| format!("gdn_job[{}:{at}]", at + bits - 1);
    replaced(
        &text,
        &[
            (
                &format!("  wire [{st}:0] g_q, g_out, g_k, g_v, g_ld, g_gt;\n", st = sa - 1),
                &format!("  wire [{st}:0] g_q, g_out, g_k, g_v, g_ld, g_gt, g_z;\n", st = sa - 1),
            ),
            (".ld_beat(g_ld), .gt_beat(g_gt));\n", &format!(".ld_beat(g_ld), .gt_beat(g_gt), .z_slot({}), .z_beat(g_z));\n", field(GdnJob::Z, 5))),
            (
                ".gt_beat(g_gt), .st_ra(g_ra),",
                &format!(
                    ".gt_beat(g_gt), .norm(gdn_job[{}]), .eps({}), .z_beat(g_z),\n    .w_line({}), .st_ra(g_ra),",
                    GdnJob::NORM,
                    field(GdnJob::EPS, 32),
                    field(GdnJob::W_LINE, 32)
                ),
            ),
        ],
    )
}

/// `logos_engine_fast` with the recurrence unit ([`fast_recurrence`]'s `text`) and the vector unit's side beside the
/// streaming unit ([`Streaming::vectors`]): `logos_vector_side` started the cycle after the middle of the engine cycle
/// the core hands it a vector job in (`vec_go`), its tables written as the core's are, the store and the recurrence
/// unit's port its from then to the next matvec or recurrence job's start, and its busy held as the other units' are.
fn fast_vectors(text: &str, p: &EngineParams) -> String {
    let sa = store_address_bits(p);
    let unit = format!(
        r#"  // The vector unit's side: a vector job on the fast side's vector unit, started the cycle after the middle of the
  // engine cycle the core hands it over in (vec_go); the store and the recurrence unit's port its from then to the next
  // matvec or recurrence job's start; its tables written at the core's step, as the core's are.
  (* maxfan = 256 *) reg vec;
  always @(posedge clk)
    if (rst) vec <= 1'b0;
    else if (start || g_start) vec <= 1'b0;
    else if (v_start) vec <= 1'b1;
  wire v_busy, v_we, v_rq_valid, v_wq_valid;
  wire [{st}:0] v_ra, v_wa;
  wire [511:0] v_wd, v_wq_data;
  wire [63:0] v_wm;
  wire [31:0] v_rq_line, v_wq_line;
  logos_vector_side #(.ROW_MAX({rows}), .VROWS({vrows}), .PROWS({prows}), .FU({fu}), .JOB_WORDS({jobs}), .TENSORS({tensors}),
    .LAYERS({layers}), .ROPES({ropes}), .SA({sa}), .FM_ROWS({fm_rows}), .PASSES({passes}), .DEPTH({depth})) vside(.clk(clk), .rst(rst), .start(v_start), .job_pc(vec_job[{pc_top}:{pc}]),
    .rows(vec_job[{rows_top}:{rows_at}]), .state_f16(vec_job[{f16}]), .busy(v_busy), .port_fault(gdn_fault), .fault(vec_fault),
    .host_we(ce && host_wr), .host_table(host_table), .host_addr(host_addr), .host_wdata(host_wdata), .st_ra(v_ra), .st_rd(rd),
    .st_we(v_we), .st_wa(v_wa), .st_wd(v_wd), .st_wm(v_wm), .rq_valid(v_rq_valid), .rq_ready(vec && p_rq_ready),
    .rq_line(v_rq_line), .rs_valid(vec && p_rs_valid), .rs_data(rs_data), .wq_valid(v_wq_valid), .wq_ready(vec && p_wq_ready),
    .wq_line(v_wq_line), .wq_data(v_wq_data), .written(!writing));
  assign rq_ready = !vec && p_rq_ready;
  assign rs_valid = !vec && p_rs_valid;
  assign wq_ready = !vec && p_wq_ready;
"#,
        st = sa - 1,
        rows = p.rows,
        vrows = p.side_rows,
        prows = p.pair_rows.min(p.side_rows),
        fu = p.function_units,
        jobs = p.job_words,
        tensors = p.tensors.max(1),
        layers = p.layers.max(1),
        ropes = p.ropes.max(1),
        fm_rows = p.streaming.as_ref().map_or(super::vector_side::FM_ROWS, |s| s.float_rows),
        passes = p.streaming.as_ref().map_or(1, |s| s.vector_passes),
        depth = p.streaming.as_ref().map_or(1, |s| s.vector_depth),
        pc = VectorSideJob::PC,
        pc_top = VectorSideJob::PC + 31,
        rows_at = VectorSideJob::ROWS,
        rows_top = VectorSideJob::ROWS + 15,
        f16 = VectorSideJob::F16,
    );
    replaced(
        text,
        &[
            (
                "// A hybrid engine's user-clock side: the streaming unit, the recurrence unit and the token-row store, the store the\n\
                 // units' while they own it (own), the core's otherwise, read the cycle after the engine cycle's middle (cr) and\n\
                 // written at its step (ce).\n",
                "// A hybrid engine's user-clock side: the streaming unit, the recurrence unit, the vector unit's side and the token-row\n\
                 // store, the store the units' while they own it (own), the core's otherwise, read the cycle after the engine cycle's\n\
                 // middle (cr) and written at its step (ce).\n",
            ),
            (
                "   input [31:0] map_space,",
                &format!(
                    "   input vec_go, input [{v}:0] vec_job, output vec_fault,\n   input [31:0] map_space,",
                    v = VectorSideJob::BITS - 1
                ),
            ),
            ("  wire g_start = gdn_go && cr;\n", "  wire g_start = gdn_go && cr;\n  wire v_start = vec_go && cr;\n"),
            ("    else if (start) gdn <= 1'b0;\n", "    else if (start || v_start) gdn <= 1'b0;\n"),
            ("  wire g_busy, g_we, rq_valid, rq_ready, rs_valid, wq_valid, wq_ready, writing;\n", "  wire g_busy, g_we, rq_valid, rq_ready, rs_valid, wq_valid, wq_ready, writing, p_rq_ready, p_rs_valid, p_wq_ready;\n"),
            (
                ".clear(g_start), .fault(gdn_fault), .rq_valid(rq_valid), .rq_ready(rq_ready),\n    \
                 .rq_line(rq_line), .rs_valid(rs_valid), .rs_data(rs_data), .wq_valid(wq_valid), .wq_ready(wq_ready), .wq_line(wq_line),\n    \
                 .wq_data(wq_data), .writing(writing),",
                ".clear(g_start || v_start), .fault(gdn_fault), .rq_valid(vec ? v_rq_valid : rq_valid),\n    \
                 .rq_ready(p_rq_ready), .rq_line(vec ? v_rq_line : rq_line), .rs_valid(p_rs_valid), .rs_data(rs_data),\n    \
                 .wq_valid(vec ? v_wq_valid : wq_valid), .wq_ready(p_wq_ready), .wq_line(vec ? v_wq_line : wq_line),\n    \
                 .wq_data(vec ? v_wq_data : wq_data), .writing(writing),",
            ),
            ("  assign md_valid = gdn ? p_md_valid : u_md_valid;\n", "  assign md_valid = (gdn || vec) ? p_md_valid : u_md_valid;\n"),
            ("  assign md_data = gdn ? p_md_data : u_md_data;\n", "  assign md_data = (gdn || vec) ? p_md_data : u_md_data;\n"),
            ("  assign mr_ready = gdn ? p_mr_ready : u_mr_ready;\n", "  assign mr_ready = (gdn || vec) ? p_mr_ready : u_mr_ready;\n"),
            ("  logos_fast_port #(", &format!("{unit}  logos_fast_port #(")),
            (
                "    else if (cr) busy_held <= busy || start || g_busy || writing || g_start;\n",
                "    else if (cr) busy_held <= busy || start || g_busy || writing || g_start || v_busy || v_start;\n",
            ),
            (
                ".re(own || cr), .ra(own ? (gdn ? g_ra : u_ra) : e_ra), .rd(rd),\n    \
                 .we(own ? (gdn ? g_we : u_we) : ce && e_we), .wa(own ? (gdn ? g_wa : u_wa) : e_wa),\n    \
                 .wd(own ? (gdn ? g_wd : u_wd) : e_wd), .wm(own ? (gdn ? {64{1'b1}} : u_wm) : e_wm));\n",
                ".re(own || cr), .ra(own ? (vec ? v_ra : gdn ? g_ra : u_ra) : e_ra), .rd(rd),\n    \
                 .we(own ? (vec ? v_we : gdn ? g_we : u_we) : ce && e_we), .wa(own ? (vec ? v_wa : gdn ? g_wa : u_wa) : e_wa),\n    \
                 .wd(own ? (vec ? v_wd : gdn ? g_wd : u_wd) : e_wd), .wm(own ? (vec ? v_wm : gdn ? {64{1'b1}} : u_wm) : e_wm));\n",
            ),
        ],
    )
}

/// `logos_engine_fast` with the vector unit's side ([`fast_vectors`]'s text) and the attention side beside it
/// ([`Streaming::attention`]): `logos_attend_side` on the recurrence unit's port, started the cycle after the middle of
/// the engine cycle the core hands it a token-row in (`att_go`); the store and the port its from then to the next
/// matvec, recurrence or vector job's start; it only reads the card.
fn fast_attention(text: &str, p: &EngineParams) -> String {
    let sa = store_address_bits(p);
    let (hd, vd, ctx, _) = p.attend.expect("an attention side attends a model's attention blocks");
    let field = |at: usize, bits: usize| format!("att_job[{}:{at}]", at + bits - 1);
    let unit = format!(
        r#"  wire a_busy, a_we, a_rq_valid;
  wire [{st}:0] a_ra, a_wa;
  wire [511:0] a_wd;
  wire [31:0] a_rq_line;
  logos_attend_side #(.HD({hd}), .VD({vd}), .P_MAX({ctx}), .SA({sa})) aside(.clk(clk), .rst(rst), .start(a_start), .busy(a_busy),
    .region({region}), .keys({keys}), .values({values}), .kscales({kscales}), .vscales({vscales}), .kv_heads({kv}),
    .heads({heads}), .positions({positions}), .scale({scale}), .q_beat({q}), .out_beat({o}), .st_ra(a_ra), .st_rd(rd),
    .st_we(a_we), .st_wa(a_wa), .st_wd(a_wd), .rq_valid(a_rq_valid), .rq_ready(att && p_rq_ready), .rq_line(a_rq_line),
    .rs_valid(att && p_rs_valid), .rs_data(rs_data));
"#,
        st = sa - 1,
        region = field(AttendJob::REGION, 32),
        keys = field(AttendJob::KEYS, 32),
        values = field(AttendJob::VALUES, 32),
        kscales = field(AttendJob::KEY_SCALES, 32),
        vscales = field(AttendJob::VALUE_SCALES, 32),
        kv = field(AttendJob::KV_HEADS, 32),
        heads = field(AttendJob::HEADS, 16),
        positions = field(AttendJob::POSITIONS, 32),
        scale = field(AttendJob::SCALE, 32),
        q = field(AttendJob::QUERY, sa),
        o = field(AttendJob::OUTPUT, sa),
    );
    replaced(
        text,
        &[
            (
                "// A hybrid engine's user-clock side: the streaming unit, the recurrence unit, the vector unit's side and the token-row\n\
                 // store, the store the units' while they own it (own), the core's otherwise, read the cycle after the engine cycle's\n\
                 // middle (cr) and written at its step (ce).\n",
                "// A hybrid engine's user-clock side: the streaming unit, the recurrence unit, the vector unit's side, the attention side\n\
                 // and the token-row store, the store the units' while they own it (own), the core's otherwise, read the cycle after\n\
                 // the engine cycle's middle (cr) and written at its step (ce).\n",
            ),
            ("   input [31:0] map_space,", &format!("   input att_go, input [{}:0] att_job,\n   input [31:0] map_space,", AttendJob::BITS - 1)),
            (
                "  wire v_start = vec_go && cr;\n",
                "  wire v_start = vec_go && cr;\n  wire a_start = att_go && cr;\n  \
                 // The attention side's: a token-row of an attention job, from the cycle after the middle of the engine cycle the\n  \
                 // core hands it over in (att_go) to the next unit's start.\n  \
                 (* maxfan = 256 *) reg att;\n  \
                 always @(posedge clk)\n    \
                 if (rst) att <= 1'b0;\n    \
                 else if (start || g_start || v_start) att <= 1'b0;\n    \
                 else if (a_start) att <= 1'b1;\n",
            ),
            ("    else if (start || v_start) gdn <= 1'b0;\n", "    else if (start || v_start || a_start) gdn <= 1'b0;\n"),
            ("    else if (start || g_start) vec <= 1'b0;\n", "    else if (start || g_start || a_start) vec <= 1'b0;\n"),
            ("  assign rq_ready = !vec && p_rq_ready;\n", "  assign rq_ready = !vec && !att && p_rq_ready;\n"),
            ("  assign rs_valid = !vec && p_rs_valid;\n", "  assign rs_valid = !vec && !att && p_rs_valid;\n"),
            ("  assign wq_ready = !vec && p_wq_ready;\n", "  assign wq_ready = !vec && !att && p_wq_ready;\n"),
            (
                ".clear(g_start || v_start), .fault(gdn_fault), .rq_valid(vec ? v_rq_valid : rq_valid),\n    \
                 .rq_ready(p_rq_ready), .rq_line(vec ? v_rq_line : rq_line), .rs_valid(p_rs_valid), .rs_data(rs_data),\n    \
                 .wq_valid(vec ? v_wq_valid : wq_valid),",
                ".clear(g_start || v_start || a_start), .fault(gdn_fault), .rq_valid(att ? a_rq_valid : vec ? v_rq_valid : rq_valid),\n    \
                 .rq_ready(p_rq_ready), .rq_line(att ? a_rq_line : vec ? v_rq_line : rq_line), .rs_valid(p_rs_valid), .rs_data(rs_data),\n    \
                 .wq_valid(!att && (vec ? v_wq_valid : wq_valid)),",
            ),
            ("  assign md_valid = (gdn || vec) ? p_md_valid : u_md_valid;\n", "  assign md_valid = (gdn || vec || att) ? p_md_valid : u_md_valid;\n"),
            ("  assign md_data = (gdn || vec) ? p_md_data : u_md_data;\n", "  assign md_data = (gdn || vec || att) ? p_md_data : u_md_data;\n"),
            ("  assign mr_ready = (gdn || vec) ? p_mr_ready : u_mr_ready;\n", "  assign mr_ready = (gdn || vec || att) ? p_mr_ready : u_mr_ready;\n"),
            ("  logos_fast_port #(", &format!("{unit}  logos_fast_port #(")),
            (
                "    else if (cr) busy_held <= busy || start || g_busy || writing || g_start || v_busy || v_start;\n",
                "    else if (cr) busy_held <= busy || start || g_busy || writing || g_start || v_busy || v_start || a_busy || a_start;\n",
            ),
            (
                ".re(own || cr), .ra(own ? (vec ? v_ra : gdn ? g_ra : u_ra) : e_ra), .rd(rd),\n    \
                 .we(own ? (vec ? v_we : gdn ? g_we : u_we) : ce && e_we), .wa(own ? (vec ? v_wa : gdn ? g_wa : u_wa) : e_wa),\n    \
                 .wd(own ? (vec ? v_wd : gdn ? g_wd : u_wd) : e_wd), .wm(own ? (vec ? v_wm : gdn ? {64{1'b1}} : u_wm) : e_wm));\n",
                ".re(own || cr), .ra(own ? (att ? a_ra : vec ? v_ra : gdn ? g_ra : u_ra) : e_ra), .rd(rd),\n    \
                 .we(own ? (att ? a_we : vec ? v_we : gdn ? g_we : u_we) : ce && e_we),\n    \
                 .wa(own ? (att ? a_wa : vec ? v_wa : gdn ? g_wa : u_wa) : e_wa),\n    \
                 .wd(own ? (att ? a_wd : vec ? v_wd : gdn ? g_wd : u_wd) : e_wd),\n    \
                 .wm(own ? (att ? {64{1'b1}} : vec ? v_wm : gdn ? {64{1'b1}} : u_wm) : e_wm));\n",
            ),
        ],
    )
}

/// `logos_engine_fast` ([`fast_verilog`]'s `text`) with the recurrence unit beside the streaming unit
/// ([`Streaming::recurrence`]): `logos_gdn_side` on the channels through `logos_fast_port`, started the cycle after
/// the middle of the engine cycle the core gives it a token-row in (`gdn_go`), the store and the channels its from then
/// to the next matvec job's start; the unit's busy and the port's writes not yet reported written held as the matvec
/// unit's busy is, so the core takes the store and the channels back only once the row's state is on the card.
fn fast_recurrence(text: &str, p: &EngineParams, s: &Streaming) -> String {
    use super::stream::{AW, LW};
    let (hd, flush) = p.delta.expect("a recurrence unit runs a model's Gated-DeltaNet blocks");
    let (t_max, window) = recurrence_log(flush);
    assert!(window.is_none() || s.recurrence_lanes > 1, "a trailing flush runs on the recurrence's head lanes alone");
    let sa = store_address_bits(p);
    let (mdt, dw) = (4 * (AW + LW + 1) - 1, 8 * s.layout.line_bytes);
    let field = |at: usize, bits: usize| format!("gdn_job[{}:{at}]", at + bits - 1);
    let unit = format!(
        r#"  // The recurrence unit: a lazily flushed memory's token-rows on the channels through the fast port, the job started the
  // cycle after the middle of the engine cycle the core gives it the job in (gdn_go); the store and the channels the
  // unit's from then to the next matvec job's start. The job's rows given on in order (logos_gdn_rows), each laid from
  // the rows and slots tables, the side taking the next once it is idle and the last row's writes are on the card (the
  // next row may be the same sequence's); the unit busy while a row is left.
  wire g_start = gdn_go && cr;
  reg gdn;
  always @(posedge clk)
    if (rst) gdn <= 1'b0;
    else if (start) gdn <= 1'b0;
    else if (g_start) gdn <= 1'b1;
  wire g_busy, g_we, rq_valid, rq_ready, rs_valid, wq_valid, wq_ready, writing;
  wire [{st}:0] g_ra, g_wa;
  wire [511:0] g_wd;
  wire [{dt}:0] rs_data, wq_data;
  wire [31:0] rq_line, wq_line;
  wire [3:0] p_md_valid, p_mr_ready;
  wire [{mdt}:0] p_md_data;
  wire g_row, g_take, g_side_busy;
  wire [31:0] g_region;
  wire [{st}:0] g_q, g_out, g_k, g_v, g_ld, g_gt;
  logos_gdn_rows #(.SA({sa}), .ROW_MAX({row_max})) rows_of(.clk(clk), .rst(rst), .start(g_start), .rows({rows}),
    .q_slot({q_slot}), .out_slot({out_slot}), .k_slot({k_slot}), .v_slot({v_slot}), .v_at({v_at}), .ld_slot({ld_slot}),
    .gt_slot({gt_slot}), .host_we(ce && host_wr), .host_table(host_table), .host_addr(host_addr), .host_wdata(host_wdata),
    .row_valid(g_row), .row_ready(g_take), .region(g_region), .q_beat(g_q), .out_beat(g_out), .k_beat(g_k), .v_beat(g_v),
    .ld_beat(g_ld), .gt_beat(g_gt));
  reg g_took;
  always @(posedge clk) g_took <= !rst && g_take;
  assign g_take = g_row && !g_side_busy && !g_took && !writing;
  assign g_busy = g_side_busy || g_row;
  logos_gdn_side #(.HD({hd}), .TMAX({t_max}), .SA({sa}), .W({w})) side(.clk(clk), .rst(rst), .start(g_take), .busy(g_side_busy),
    .region(g_region), .mem_off({memory}), .log_off({log}), .entry({entry}), .nk({nk}), .nv({nv}), .fval({fval}),
    .fdec({fdec}), .fgate({fgate}), .scale({scale}), .tokens({tokens}), .f16(gdn_job[{f16}]), .q_beat(g_q),
    .out_beat(g_out), .k_beat(g_k), .v_beat(g_v), .ld_beat(g_ld), .gt_beat(g_gt), .st_ra(g_ra),
    .st_rd(rd), .st_we(g_we), .st_wa(g_wa), .st_wd(g_wd), .rq_valid(rq_valid), .rq_ready(rq_ready), .rq_line(rq_line),
    .rs_valid(rs_valid), .rs_data(rs_data), .wq_valid(wq_valid), .wq_ready(wq_ready), .wq_line(wq_line), .wq_data(wq_data),
    .written(!writing));
  logos_fast_port #(.NB(4), .AW({aw}), .LW({lw}), .DL({dw})) port(.clk(clk), .rst(rst), .space(map_space), .base(map_base),
    .granule_log2(map_granule_log2), .clear(g_start), .fault(gdn_fault), .rq_valid(rq_valid), .rq_ready(rq_ready),
    .rq_line(rq_line), .rs_valid(rs_valid), .rs_data(rs_data), .wq_valid(wq_valid), .wq_ready(wq_ready), .wq_line(wq_line),
    .wq_data(wq_data), .writing(writing), .md_valid(p_md_valid), .md_ready(md_ready), .md_data(p_md_data),
    .mr_valid(mr_valid), .mr_ready(p_mr_ready), .mr_data(mr_data), .mw_valid(mw_valid), .mw_ready(mw_ready),
    .mw_data(mw_data), .mdone_valid(mdone_valid), .mdone_ready(mdone_ready));
  assign md_valid = gdn ? p_md_valid : u_md_valid;
  assign md_data = gdn ? p_md_data : u_md_data;
  assign mr_ready = gdn ? p_mr_ready : u_mr_ready;
"#,
        st = sa - 1,
        dt = dw - 1,
        w = s.recurrence_width,
        aw = AW,
        lw = LW,
        rows = field(GdnJob::ROWS, 16),
        row_max = p.rows,
        memory = field(GdnJob::MEMORY, 32),
        log = field(GdnJob::LOG, 32),
        entry = field(GdnJob::ENTRY, 32),
        nk = field(GdnJob::KEY_HEADS, 32),
        nv = field(GdnJob::VALUE_HEADS, 32),
        fval = field(GdnJob::VALUES, 32),
        fdec = field(GdnJob::DECAYS, 32),
        fgate = field(GdnJob::GATES, 32),
        scale = field(GdnJob::SCALE, 32),
        q_slot = field(GdnJob::QUERY, 5),
        out_slot = field(GdnJob::OUTPUT, 5),
        k_slot = field(GdnJob::KEY, 5),
        v_slot = field(GdnJob::VALUE, 5),
        v_at = field(GdnJob::VALUE_AT, 20),
        ld_slot = field(GdnJob::LOG_DECAY, 5),
        gt_slot = field(GdnJob::GATE, 5),
        tokens = field(GdnJob::TOKENS, 8),
        f16 = GdnJob::F16,
    );
    replaced(
        text,
        &[
            (
                "// A hybrid engine's user-clock side: the streaming unit and the token-row store, the store the unit's while it owns it\n\
                 // (own), the core's otherwise, read the cycle after the engine cycle's middle (cr) and written at its step (ce).\n",
                "// A hybrid engine's user-clock side: the streaming unit, the recurrence unit and the token-row store, the store the\n\
                 // units' while they own it (own), the core's otherwise, read the cycle after the engine cycle's middle (cr) and\n\
                 // written at its step (ce).\n",
            ),
            (
                &format!("   input [3:0] mr_valid, output [3:0] mr_ready, input [{}:0] mr_data);\n", 4 * dw - 1),
                &format!(
                    "   input [3:0] mr_valid, output [3:0] mr_ready, input [{d}:0] mr_data,\n   \
                     output [3:0] mw_valid, input [3:0] mw_ready, output [{d}:0] mw_data, input [3:0] mdone_valid, output [3:0] mdone_ready,\n   \
                     input gdn_go, input [{g}:0] gdn_job, output gdn_fault,\n   \
                     input host_wr, input [2:0] host_table, input [31:0] host_addr, input [511:0] host_wdata,\n   \
                     input [31:0] map_space, input [{a}:0] map_base, input [4:0] map_granule_log2);\n",
                    d = 4 * dw - 1,
                    g = GdnJob::bits(s) - 1,
                    a = AW - 1
                ),
            ),
            (
                "  wire busy, u_we;\n",
                &format!("  wire busy, u_we;\n  wire [3:0] u_md_valid, u_mr_ready;\n  wire [{mdt}:0] u_md_data;\n"),
            ),
            (
                ".md_valid(md_valid), .md_ready(md_ready), .md_data(md_data), .mr_valid(mr_valid),\n    .mr_ready(mr_ready), .mr_data(mr_data));\n",
                &format!(
                    ".md_valid(u_md_valid), .md_ready(md_ready), .md_data(u_md_data), .mr_valid(mr_valid),\n    \
                     .mr_ready(u_mr_ready), .mr_data(mr_data));\n{unit}"
                ),
            ),
            ("    else if (cr) busy_held <= busy || start;\n", "    else if (cr) busy_held <= busy || start || g_busy || writing || g_start;\n"),
            (
                ".re(own || cr), .ra(own ? u_ra : e_ra), .rd(rd),\n    .we(own ? u_we : ce && e_we), .wa(own ? u_wa : e_wa), .wd(own ? u_wd : e_wd), .wm(own ? u_wm : e_wm));\n",
                ".re(own || cr), .ra(own ? (gdn ? g_ra : u_ra) : e_ra), .rd(rd),\n    \
                 .we(own ? (gdn ? g_we : u_we) : ce && e_we), .wa(own ? (gdn ? g_wa : u_wa) : e_wa),\n    \
                 .wd(own ? (gdn ? g_wd : u_wd) : e_wd), .wm(own ? (gdn ? {64{1'b1}} : u_wm) : e_wm));\n",
            ),
        ],
    )
}

/// `logos_engine_fast` ([`fast_verilog`]'s `text`, every unit on the fast side) for an engine of
/// [`Streaming::contexts`] contexts: each context's core hands its jobs over as one context's does, and each class of
/// units serves one context at a time — the matvec class (the streaming unit) and the row class (the recurrence, vector
/// and attention sides, which share the fast port) — so one context's weights stream while another's rows run. A core's
/// hand-off at `cr` is its context's request of the job's class; the class grants a requesting context, the one after
/// the context it served last, at a `cr` it is idle on and no longer held: a class holds the context it serves until
/// that context's core takes its store bank back (`own`), which it does only once its busy has fallen. Each context's
/// busy holds while its request waits and while the class serving it is busy. The store is a bank a context, each
/// laid by the same layout over that context's token-rows: the matvec unit's while it serves the bank's context, the row
/// class's likewise, the context's core's otherwise. The streaming unit and the row class's ports share the channels a
/// descriptor at a time (`logos_channel_mux`, the row class port 0), the unit through a `logos_read_credit` a channel:
/// its channels' feeds step together, so it takes a channel's lines only as its other channels let it, and asking only
/// for lines a credit can hold it takes each line as it comes, as the multiplexer's order needs; the per-row units' mode
/// is set and cleared only by their own starts; the vector side reads the rows table of the context it serves; the row class's faults are held
/// for the context they arose in.
fn fast_contexts(text: &str, p: &EngineParams, s: &Streaming) -> String {
    // A streaming unit whose cores are paired runs their processing elements in clk2, twice the fast side's clock.
    let clk2_in = if s.set.pe_pairs { " input clk2," } else { "" };
    use super::stream::{AW, LW};
    let c = s.contexts;
    let cb = if c > 1 { (usize::BITS - (c - 1).leading_zeros()) as usize } else { 1 };
    let sa = store_address_bits(p);
    let (j, g, v, a) = (HybridJob::BITS, GdnJob::bits(s), VectorSideJob::BITS, AttendJob::BITS);
    let (mdw, dw) = (AW as usize + LW as usize + 1, 8 * s.layout.line_bytes as usize);
    // A context's job word among every context's: a choice among constant slices, where a slice at a stride of the
    // word's width would be a multiply and a barrel shift.
    let pick = |words: &str, width: usize, at: &str| {
        (0..c - 1).rev().fold(format!("{words}[{} +: {width}]", width * (c - 1)), |rest, k| format!("({at} == {cb}'d{k}) ? {words}[{} +: {width}] : {rest}", width * k))
    };
    let head = text.find("module logos_engine_fast\n").expect("the fast side's module");
    let body_at = head + text[head..].find(");\n").expect("the fast side's ports") + 3;
    let body = replaced(
        &text[body_at..],
        &[
            ("  wire start = go && cr;\n", "@CONTEXT_WIRES@"),
            ("  wire g_start = gdn_go && cr;\n", ""),
            ("  wire v_start = vec_go && cr;\n", ""),
            ("  wire a_start = att_go && cr;\n", ""),
            ("    else if (start || g_start || v_start) att <= 1'b0;\n", "    else if (g_start || v_start) att <= 1'b0;\n"),
            ("    else if (start || v_start || a_start) gdn <= 1'b0;\n", "    else if (v_start || a_start) gdn <= 1'b0;\n"),
            ("    else if (start || g_start || a_start) vec <= 1'b0;\n", "    else if (g_start || a_start) vec <= 1'b0;\n"),
            ("  assign md_valid = (gdn || vec || att) ? p_md_valid : u_md_valid;\n", ""),
            ("  assign md_data = (gdn || vec || att) ? p_md_data : u_md_data;\n", ""),
            ("  assign mr_ready = (gdn || vec || att) ? p_mr_ready : u_mr_ready;\n", ""),
            (
                "  always @(posedge clk)\n    if (rst) busy_held <= 1'b0;\n    \
                 else if (cr) busy_held <= busy || start || g_busy || writing || g_start || v_busy || v_start || a_busy || a_start;\n",
                "",
            ),
        ],
    );
    let body = spliced(&body, "  logos_bram_re_w512 #(.D(", "endmodule\n", "@CONTEXT_GRANTS@endmodule\n");
    let mut body = body;
    for name in ["md_valid", "md_ready", "md_data", "mr_valid", "mr_ready", "mr_data", "mw_valid", "mw_ready", "mw_data", "mdone_valid", "mdone_ready"] {
        body = renamed(&body, name, &format!("x_{name}"));
    }
    for (from, to) in [("job", "mjob"), ("gdn_job", "gjob"), ("vec_job", "vjob"), ("att_job", "ajob"), ("gdn_fault", "g_fault"), ("vec_fault", "v_fault"), ("rd", "w_rd")] {
        body = renamed(&body, from, to);
    }
    let unit = body.find(" unit(.clk(clk)").expect("the streaming unit's instance");
    let unit_end = unit + body[unit..].find(");\n").expect("its end");
    let instance = replaced(
        &body[unit..unit_end],
        &[
            (".md_ready(x_md_ready)", ".md_ready(u_md_ready)"),
            (".mr_valid(x_mr_valid)", ".mr_valid(u_mr_valid)"),
            (".mr_data(x_mr_data)", ".mr_data(u_mr_data)"),
            (".st_rd(w_rd)", ".st_rd(u_rd)"),
        ],
    );
    let body = format!("{}{instance}{}", &body[..unit], &body[unit_end..]);
    let body = replaced(
        &body,
        &[
            ("  logos_vector_side #(", &format!("  logos_vector_side #(.CTX({c}), .CB({cb}), ")),
            (".port_fault(g_fault), .fault(v_fault),", ".port_fault(g_fault), .fault(v_fault), .ctx(rw_at),"),
            ("  logos_gdn_rows #(", &format!("  logos_gdn_rows #(.CTX({c}), .CB({cb}), ")),
            (".rst(rst), .start(g_start), .rows(", ".rst(rst), .ctx(rw_at), .start(g_start), .rows("),
        ],
    );
    let wires = format!(
        r#"  // The units' starts and the jobs they take, from the contexts' grants below.
  wire start, g_start, v_start, a_start;
  wire [{jt}:0] mjob;
  wire [{gt}:0] gjob;
  wire [{vt}:0] vjob;
  wire [{at}:0] ajob;
  wire [511:0] u_rd, w_rd;
  wire [{cbt}:0] rw_at;
  wire g_fault, v_fault;
  wire [3:0] u_md_ready, u_mr_valid, x_md_ready, x_mr_valid, x_mw_valid, x_mw_ready, x_mdone_valid, x_mdone_ready;
  wire [{dt}:0] x_mr_data, x_mw_data, u_mr_data;
  wire [3:0] k_md_valid, k_md_ready, k_mr_valid, k_mr_ready, pw_ready_unused, pdone_valid_unused;
  wire [4*({aw}+{lw}+1)-1:0] k_md_data;
"#,
        jt = j - 1,
        gt = g - 1,
        vt = v - 1,
        at = a - 1,
        cbt = cb - 1,
        dt = 4 * dw - 1,
        aw = AW,
        lw = LW,
    );
    // The channels a descriptor at a time, every port straight onto one multiplexer: with head lanes their multiplexer,
    // the streaming unit's credits its last port; else the row class's port 0 and the credits' port 1.
    let lanes_mux = "  logos_channel_mux #(.P(";
    let lanes = body.find(lanes_mux).map(|at| (at, at + body[at..].find(");\n").expect("the lanes' multiplexer's end") + 3));
    let (body, channels) = match lanes {
        Some((at, end)) => {
            let ports: usize = body[at + lanes_mux.len()..].split(')').next().and_then(|n| n.parse().ok()).expect("the lanes' multiplexer's ports");
            let flat = format!(
                "  logos_channel_mux #(.P({}), .NB(4), .AW({AW}), .LW({LW}), .DL({dw})) mux(.clk(clk), .rst(rst),\n    \
                 .pd_valid({{k_md_valid, c_md_valid}}), .pd_ready({{k_md_ready, c_md_ready}}), .pd_data({{k_md_data, c_md_data}}),\n    \
                 .pr_valid({{k_mr_valid, c_mr_valid}}), .pr_ready({{k_mr_ready, c_mr_ready}}), .pw_valid({{4'd0, c_mw_valid}}),\n    \
                 .pw_ready({{pw_ready_unused, c_mw_ready}}), .pw_data({{{}'d0, c_mw_data}}), .pdone_valid({{pdone_valid_unused, c_mdone_valid}}),\n    \
                 .pdone_ready({{4'hf, c_mdone_ready}}), .md_valid(md_valid), .md_ready(md_ready), .md_data(md_data), .mr_valid(mr_valid),\n    \
                 .mr_ready(mr_ready), .mw_valid(mw_valid), .mw_ready(mw_ready), .mw_data(mw_data), .mdone_valid(mdone_valid),\n    \
                 .mdone_ready(mdone_ready));\n",
                ports + 1,
                4 * dw
            );
            (format!("{}{flat}{}", &body[..at], &body[end..]), String::new())
        }
        None => (
            body,
            format!(
                "  logos_channel_mux #(.P(2), .NB(4), .AW({AW}), .LW({LW}), .DL({dw})) cmux(.clk(clk), .rst(rst),\n    \
                 .pd_valid({{k_md_valid, p_md_valid}}), .pd_ready({{k_md_ready, x_md_ready}}), .pd_data({{k_md_data, p_md_data}}),\n    \
                 .pr_valid({{k_mr_valid, x_mr_valid}}), .pr_ready({{k_mr_ready, p_mr_ready}}), .pw_valid({{4'd0, x_mw_valid}}),\n    \
                 .pw_ready({{pw_ready_unused, x_mw_ready}}), .pw_data({{{}'d0, x_mw_data}}), .pdone_valid({{pdone_valid_unused, x_mdone_valid}}),\n    \
                 .pdone_ready({{4'hf, x_mdone_ready}}), .md_valid(md_valid), .md_ready(md_ready), .md_data(md_data), .mr_valid(mr_valid),\n    \
                 .mr_ready(mr_ready), .mw_valid(mw_valid), .mw_ready(mw_ready), .mw_data(mw_data), .mdone_valid(mdone_valid),\n    \
                 .mdone_ready(mdone_ready));\n",
                4 * dw
            ),
        ),
    };
    let grants = format!(
        r#"  // The contexts' grants: a class grants the first requesting context after the one it served last.
  function [{cb}:0] next_of(input [{ct}:0] want, input [{cbt}:0] last);
    integer i, k;
    reg found;
    reg [{cbt}:0] at;
    begin
      found = 1'b0;
      at = last;
      for (i = 1; i <= {c}; i = i + 1) begin
        k = (last + i) % {c};
        if (!found && want[k]) begin found = 1'b1; at = k[{cbt}:0]; end
      end
      next_of = {{found, at}};
    end
  endfunction
  // The matvec class.
  reg [{ct}:0] mv_pend;
  reg [{cbt}:0] mv_ctx;
  reg mv_valid;
  wire [{ct}:0] mv_ask = go & {{{c}{{cr}}}};
  wire [{cb}:0] mv_next = next_of(mv_pend | mv_ask, mv_ctx);
  wire mv_grant = cr && mv_next[{cb}] && !busy && !(mv_valid && own[mv_ctx]);
  wire [{cbt}:0] mv_at = mv_grant ? mv_next[{cbt}:0] : mv_ctx;
  assign start = mv_grant;
  assign mjob = {mjob_sel};
  always @(posedge clk)
    if (rst) begin mv_pend <= {{{c}{{1'b0}}}}; mv_ctx <= {{{cb}{{1'b0}}}}; mv_valid <= 1'b0; end
    else begin
      mv_pend <= (mv_pend | mv_ask) & ~({{{{{cm}{{1'b0}}}}, mv_grant}} << mv_next[{cbt}:0]);
      if (mv_grant) begin mv_ctx <= mv_next[{cbt}:0]; mv_valid <= 1'b1; end
      else if (mv_valid && !own[mv_ctx]) mv_valid <= 1'b0;
    end
  // The row class: a request's kind, 0 a recurrence row, 1 a vector job, 2 an attention row.
  reg [{ct}:0] rw_pend;
  reg [{kt}:0] rw_kinds;
  (* maxfan = 256 *) reg [{cbt}:0] rw_ctx;
  reg rw_valid;
  reg [1:0] rw_kind;
  wire [{ct}:0] rw_ask = (gdn_go | vec_go | att_go) & {{{c}{{cr}}}};
  wire [{cb}:0] rw_next = next_of(rw_pend | rw_ask, rw_ctx);
  wire [{cbt}:0] rw_to = rw_next[{cbt}:0];
  wire rw_busy = g_busy || writing || v_busy || a_busy;
  wire rw_grant = cr && rw_next[{cb}] && !rw_busy && !(rw_valid && own[rw_ctx]);
  wire [1:0] rw_new = gdn_go[rw_to] ? 2'd0 : vec_go[rw_to] ? 2'd1 : 2'd2;
  wire [1:0] rw_k = rw_ask[rw_to] ? rw_new : rw_kinds[2*rw_to +: 2];
  assign g_start = rw_grant && rw_k == 2'd0;
  assign v_start = rw_grant && rw_k == 2'd1;
  assign a_start = rw_grant && rw_k == 2'd2;
  assign rw_at = rw_grant ? rw_to : rw_ctx;
  assign gjob = {gjob_sel};
  assign vjob = {vjob_sel};
  assign ajob = {ajob_sel};
  integer rk;
  always @(posedge clk)
    if (rst) begin rw_pend <= {{{c}{{1'b0}}}}; rw_kinds <= {{{k2}{{1'b0}}}}; rw_ctx <= {{{cb}{{1'b0}}}}; rw_valid <= 1'b0; rw_kind <= 2'd0; end
    else begin
      for (rk = 0; rk < {c}; rk = rk + 1)
        if (rw_ask[rk]) rw_kinds[2*rk +: 2] <= gdn_go[rk] ? 2'd0 : vec_go[rk] ? 2'd1 : 2'd2;
      rw_pend <= (rw_pend | rw_ask) & ~({{{{{cm}{{1'b0}}}}, rw_grant}} << rw_to);
      if (rw_grant) begin rw_ctx <= rw_to; rw_valid <= 1'b1; rw_kind <= rw_k; end
      else if (rw_valid && !own[rw_ctx]) rw_valid <= 1'b0;
    end
  // Each context's busy: its request waiting, or the class serving it busy.
  integer bk;
  always @(posedge clk)
    if (rst) busy_held <= {{{c}{{1'b0}}}};
    else if (cr)
      for (bk = 0; bk < {c}; bk = bk + 1)
        busy_held[bk] <= mv_pend[bk] || mv_ask[bk] || rw_pend[bk] || rw_ask[bk] || (mv_valid && mv_ctx == bk[{cbt}:0] && busy)
          || (rw_valid && rw_ctx == bk[{cbt}:0] && rw_busy);
  // The row class's faults, held for the context each arose in.
  reg [{ct}:0] gf, vf;
  always @(posedge clk)
    if (rst) begin gf <= {{{c}{{1'b0}}}}; vf <= {{{c}{{1'b0}}}}; end
    else if (rw_valid) begin
      if (rw_kind == 2'd0) gf[rw_ctx] <= g_fault;
      if (rw_kind == 2'd1) vf[rw_ctx] <= v_fault;
    end
  assign gdn_fault = gf;
  assign vec_fault = vf;
  // The store, a bank a context: the matvec unit's while it serves the bank's context, the row class's likewise, the
  // context's core's while the core holds it.
  wire [{st}:0] w_ra = att ? a_ra : vec ? v_ra : g_ra;
  wire w_we = att ? a_we : vec ? v_we : g_we;
  wire [{st}:0] w_wa = att ? a_wa : vec ? v_wa : g_wa;
  wire [511:0] w_wd = att ? a_wd : vec ? v_wd : g_wd;
  wire [63:0] w_wm = att ? {{64{{1'b1}}}} : vec ? v_wm : {{64{{1'b1}}}};
  wire [{bt}:0] banked;
  genvar bank_at;
  generate for (bank_at = 0; bank_at < {c}; bank_at = bank_at + 1) begin : bank
    wire mv_here = own[bank_at] && (mv_grant ? mv_next[{cbt}:0] == bank_at : mv_valid && mv_ctx == bank_at);
    wire rw_here = own[bank_at] && (rw_grant ? rw_to == bank_at : rw_valid && rw_ctx == bank_at);
    logos_bram_re_w512 #(.D({store}), .A({sa})) store(.clk(clk), .re(own[bank_at] || cr),
      .ra(own[bank_at] ? (mv_here ? u_ra : w_ra) : e_ra[{sa}*bank_at +: {sa}]), .rd(banked[512*bank_at +: 512]),
      .we(own[bank_at] ? (mv_here ? u_we : rw_here && w_we) : ce && e_we[bank_at]),
      .wa(own[bank_at] ? (mv_here ? u_wa : w_wa) : e_wa[{sa}*bank_at +: {sa}]),
      .wd(own[bank_at] ? (mv_here ? u_wd : w_wd) : e_wd[512*bank_at +: 512]),
      .wm(own[bank_at] ? (mv_here ? u_wm : w_wm) : e_wm[64*bank_at +: 64]));
  end endgenerate
  assign rd = banked;
  assign u_rd = banked[512*mv_ctx +: 512];
  assign w_rd = banked[512*rw_ctx +: 512];
  // The streaming unit's reads through a credit a channel: its runs asked only as the credit's buffer can hold them,
  // every line taken from the multiplexer as it comes, the unit's next line a register; each line on to the unit
  // through a pipe, so the unit's take and the credit's pop each end at a register.
  genvar kc;
  generate for (kc = 0; kc < 4; kc = kc + 1) begin : credit
    wire c_valid, c_ready;
    wire [{dw}-1:0] c_data;
    logos_read_credit #(.AW({aw}), .LW({lw}), .DL({dw})) k(.clk(clk), .rst(rst), .in_valid(u_md_valid[kc]),
      .in_ready(u_md_ready[kc]), .in_data(u_md_data[({aw}+{lw}+1)*kc +: {aw}+{lw}+1]), .out_valid(c_valid),
      .out_ready(c_ready), .out_data(c_data), .md_valid(k_md_valid[kc]),
      .md_ready(k_md_ready[kc]), .md_data(k_md_data[({aw}+{lw}+1)*kc +: {aw}+{lw}+1]), .mr_valid(k_mr_valid[kc]),
      .mr_ready(k_mr_ready[kc]), .mr_data(x_mr_data[{dw}*kc +: {dw}]));
    logos_pipe #(.W({dw})) to_unit(.clk(clk), .rst_n(!rst), .in_valid(c_valid), .in_ready(c_ready), .in_data(c_data),
      .out_valid(u_mr_valid[kc]), .out_ready(u_mr_ready[kc]), .out_data(u_mr_data[{dw}*kc +: {dw}]));
  end endgenerate
{channels}  assign x_mr_data = mr_data;
"#,
        mjob_sel = pick("job", j, "mv_at"),
        gjob_sel = pick("gdn_job", g, "rw_at"),
        vjob_sel = pick("vec_job", v, "rw_at"),
        ajob_sel = pick("att_job", a, "rw_at"),
        c = c,
        cb = cb,
        ct = c - 1,
        cbt = cb - 1,
        cm = c - 1,
        kt = 2 * c - 1,
        k2 = 2 * c,
        st = sa - 1,
        sa = sa,
        bt = 512 * c - 1,
        store = p.store_beats,
        aw = AW,
        lw = LW,
        dw = dw,
    );
    let header = format!(
        r#"// A hybrid engine's user-clock side for {c} contexts: the streaming unit, the recurrence unit, the vector unit's side, the
// attention side and the token-row store a bank a context, the matvec class serving one context while the row class
// serves another (each a context's from its grant until the context's core takes its bank back), the core's bank its own
// otherwise, read the cycle after the engine cycle's middle (cr) and written at its step (ce).
module logos_engine_fast
  (input clk,{clk2_in} input rst, input ce, input cr, input [{ct}:0] own, input [{ct}:0] go, input [{cj}:0] job,
   input [{cs}:0] e_ra, input [{ct}:0] e_we, input [{cs}:0] e_wa, input [{cd}:0] e_wd, input [{cm}:0] e_wm,
   output [{cd}:0] rd, output reg [{ct}:0] busy_held,
   output [3:0] md_valid, input [3:0] md_ready, output [{mdt}:0] md_data,
   input [3:0] mr_valid, output [3:0] mr_ready, input [{drt}:0] mr_data,
   output [3:0] mw_valid, input [3:0] mw_ready, output [{drt}:0] mw_data, input [3:0] mdone_valid, output [3:0] mdone_ready,
   input [{ct}:0] gdn_go, input [{cg}:0] gdn_job, output [{ct}:0] gdn_fault,
   input [{ct}:0] vec_go, input [{cv}:0] vec_job, output [{ct}:0] vec_fault,
   input host_wr, input [2:0] host_table, input [31:0] host_addr, input [511:0] host_wdata,
   input [{ct}:0] att_go, input [{ca}:0] att_job,
   input [31:0] map_space, input [{awt}:0] map_base, input [4:0] map_granule_log2);
"#,
        c = c,
        ct = c - 1,
        cj = c * j - 1,
        cs = c * sa - 1,
        cd = 512 * c - 1,
        cm = 64 * c - 1,
        mdt = 4 * mdw - 1,
        drt = 4 * dw - 1,
        cg = c * g - 1,
        cv = c * v - 1,
        ca = c * a - 1,
        awt = AW - 1,
    );
    let body = replaced(&body, &[("@CONTEXT_WIRES@", wires.as_str()), ("@CONTEXT_GRANTS@", grants.as_str())]);
    format!("{}{header}{body}", &text[..text[..head].rfind("// A hybrid engine's user-clock side").expect("the fast side's comment")])
}

/// `logos_vector_side` for an engine of contexts ([`fast_contexts`]): a rows table a context, the context a host write
/// names in its address's top four bits, and the job's rows read from the table of the context it runs for (`ctx`).
fn vector_side_contexts(text: &str) -> String {
    // A convolution path's rows from the table of the context the job runs for too.
    let text = text.replace("  wire [127:0] cv_row_e = rowt[cv_row_k];\n", "  wire [127:0] cv_row_e = rowt[ctx * ROW_MAX + cv_row_k];\n");
    replaced(
        &text,
        &[
            ("module logos_vector_side #(parameter ROW_MAX = 1,", "module logos_vector_side #(parameter CTX = 1, CB = 1, ROW_MAX = 1,"),
            ("   input start, input [31:0] job_pc,", "   input [CB-1:0] ctx, input start, input [31:0] job_pc,"),
            ("  `LOGOS_RAMSTYLE_ASYNC(ROW_MAX) reg [127:0] rowt [0:ROW_MAX-1];\n", "  `LOGOS_RAMSTYLE_ASYNC(CTX*ROW_MAX) reg [127:0] rowt [0:CTX*ROW_MAX-1];\n"),
            ("      3'd1: rowt[host_addr] <= host_wdata[127:0];\n", "      3'd1: rowt[host_addr[31:28] * ROW_MAX + host_addr[27:0]] <= host_wdata[127:0];\n"),
            ("          row_e <= rowt[k]; slot_e <= slott[l_what[4:0]];", "          row_e <= rowt[ctx * ROW_MAX + k]; slot_e <= slott[l_what[4:0]];"),
            ("          row_e <= rowt[k]; slot_e <= slott[p_what[4:0]];", "          row_e <= rowt[ctx * ROW_MAX + k]; slot_e <= slott[p_what[4:0]];"),
            ("  always @(posedge clk) row_region <= rowt[k][31:0];\n", "  always @(posedge clk) row_region <= rowt[ctx * ROW_MAX + k][31:0];\n"),
        ],
    )
}

/// The vector unit's side `text` ([`super::vector_side::vector_side_verilog`]'s) with a convolution job's path of its own
/// ([`Streaming::with_conv_path`]): a job whose descriptors are the machine's conv job's (three ring loads, oldest first,
/// x from a slot, the kernel's four taps from one tensor at `j·C`, the output into a slot and x into the ring, the conv
/// micro-program word for word; its channels a multiple of 16, of 32 for a binary16 ring, no more than `beats` beats)
/// reads its kernel's and its layer's table words, then hands every row to `logos_conv_path` (`lanes` lanes), which
/// takes the side's store ports, its memory requests and its line queue while it runs, and the job ends as any does
/// once every write is on the card. Every other job runs on the vector unit, of [`CONV_PATH_VREGS`] registers: a job
/// whose loads, stores or instructions name a register past them faults at its first pass and runs no further.
fn vector_side_conv(text: &str, lanes: usize, beats: usize) -> String {
    use super::fp32::F32_TO_F16_LATENCY;
    use super::vector::RNE_P_LATENCY;
    let narrow = F32_TO_F16_LATENCY.max(RNE_P_LATENCY);
    let prog: Vec<u64> = super::super::vector::conv(&[0, 1, 2], 3, &[4, 5, 6, 7], 8, 9).iter().map(super::simd::encode).collect();
    assert_eq!(prog.len(), 6, "the conv micro-program's six instructions");
    let prog_ok: String = prog.iter().enumerate().map(|(i, w)| format!(" && prog[{i}] == 64'h{w:016x}")).collect();
    let text = renamed(&renamed(text, "rq_valid", "rq_valid_s"), "rq_line", "rq_line_s");
    replaced(
        &text,
        &[
            (
                "   output reg rq_valid_s, input rq_ready, output reg [31:0] rq_line_s, input rs_valid,",
                "   output rq_valid, input rq_ready, output [31:0] rq_line, input rs_valid,",
            ),
            (
                "  localparam JA = (JOB_WORDS > 1) ? $clog2(JOB_WORDS) : 1,",
                "  // The side's own memory requests, the path's while it runs.\n  reg rq_valid_s;\n  reg [31:0] rq_line_s;\n  localparam JA = (JOB_WORDS > 1) ? $clog2(JOB_WORDS) : 1,",
            ),
            ("FM_T = 6'd39, GNEXT = 6'd40;", "FM_T = 6'd39, GNEXT = 6'd40, CV_T = 6'd41, CV_GO = 6'd42, CV_RUN = 6'd43;"),
            (
                "  always @* begin\n    j_ra = pc[JA-1:0];",
                &format!(
                    r#"  // A convolution job (`cv`): the machine's conv job, its descriptors' shape (cv_shape), every row on logos_conv_path,
  // which drives the store's ports, the memory requests and the line queue while the side waits in CV_RUN. The shape is
  // a register set every clock: the descriptors are written only in DESC, the last of them the clock before DESC
  // chooses PASS, so it holds the job's own shape from PASS's first clock on.
  wire [31:0] cv_c = ld_a[0][63:32];
  reg cv_shape;
  always @(posedge clk) cv_shape <= !heads_first && heads == 32'd1 && nload == 8'd8 && nput == 8'd2 && nprog == 16'd6 &&
    ld_a[0][7:0] == 8'h30 && ld_a[1][7:0] == 8'h31 && ld_a[2][7:0] == 8'h32 &&
    ld_a[0][31:24] == 8'd0 && ld_a[1][31:24] == 8'd1 && ld_a[2][31:24] == 8'd2 &&
    ld_a[1][23:8] == ld_a[0][23:8] && ld_a[2][23:8] == ld_a[0][23:8] && ld_a[3][7:0] == 8'h03 && ld_b[3][31:0] == 32'd0 &&
    ld_a[4][7:0] == 8'h14 && ld_a[5][7:0] == 8'h15 && ld_a[6][7:0] == 8'h16 && ld_a[7][7:0] == 8'h17 &&
    ld_a[5][23:8] == ld_a[4][23:8] && ld_a[6][23:8] == ld_a[4][23:8] && ld_a[7][23:8] == ld_a[4][23:8] &&
    ld_b[4][31:0] == 32'd0 && ld_b[5][31:0] == cv_c && ld_b[6][31:0] == cv_c + cv_c && ld_b[7][31:0] == cv_c + cv_c + cv_c &&
    ld_a[1][63:32] == cv_c && ld_a[2][63:32] == cv_c && ld_a[3][63:32] == cv_c && ld_a[4][63:32] == cv_c &&
    ld_a[5][63:32] == cv_c && ld_a[6][63:32] == cv_c && ld_a[7][63:32] == cv_c &&
    pt_a[0][7:0] == 8'h09 && pt_b[0][31:0] == 32'd0 && pt_a[1][7:0] == 8'h23 && pt_a[1][23:8] == ld_a[0][23:8] &&
    pt_a[0][63:32] == cv_c && pt_a[1][63:32] == cv_c{prog_ok} &&
    cv_c[3:0] == 4'd0 && (!f16 || !cv_c[4]) && (cv_c >> 4) <= 32'd{beats};
  reg cv_start;
  // Whether a load, a store or an instruction of the job names a register past the vector unit's: each descriptor word
  // read (wd_q) and whether it is a load's or store's register word (wd_reg) or an instruction (wd_prog), checked the
  // clock after.
  reg wide, wd_reg, wd_prog;
  reg [63:0] wd_q;
  wire cv_on = state == CV_RUN;
  wire cv_busy, cv_st_we, cv_rq_valid, cv_q_v, cv_flush;
  wire [15:0] cv_row_k;
  wire [SA-1:0] cv_st_ra, cv_st_wa;
  wire [511:0] cv_st_wd, cv_q_data;
  wire [63:0] cv_q_mask;
  wire [31:0] cv_rq_line, cv_q_line;
  wire [127:0] cv_row_e = rowt[cv_row_k];
  reg [31:0] cv_kline, cv_beats, cv_xb, cv_xs, cv_ob, cv_os, cv_roff, cv_rbytes;
  reg [7:0] cv_taps;
  logos_conv_path #(.SA(SA), .CVB({beats}), .LANES({lanes}), .SRL(SRL)) cvp(.clk(clk), .rst(rst), .start(cv_start), .rows(nrows),
    .f16(f16), .k_line(cv_kline), .beats(cv_beats), .x_base(cv_xb), .x_stride(cv_xs), .o_base(cv_ob), .o_stride(cv_os),
    .ring_off(cv_roff), .ring_taps(cv_taps), .row_bytes(cv_rbytes), .row_k(cv_row_k), .row_e(cv_row_e), .settled(settled),
    .busy(cv_busy), .st_ra(cv_st_ra), .st_rd(st_rd), .st_we(cv_st_we), .st_wa(cv_st_wa), .st_wd(cv_st_wd),
    .rq_valid(cv_rq_valid), .rq_ready(rq_ready), .rq_line(cv_rq_line), .rs_valid(rs_valid), .rs_data(rs_data), .q_v(cv_q_v),
    .q_data(cv_q_data), .q_mask(cv_q_mask), .q_line(cv_q_line), .q_n(q_n), .flush(cv_flush));
  assign rq_valid = cv_on ? cv_rq_valid : rq_valid_s;
  assign rq_line = cv_on ? cv_rq_line : rq_line_s;

  always @* begin
    j_ra = pc[JA-1:0];"#
                ),
            ),
            (
                "    if (state == FM_ACT) st_ra = fm_from[SA-1:0] + fm_ar[SA-1:0] * fm_from[SA-1+32:32] + fm_at[SA-1:0];\n",
                "    if (state == FM_ACT) st_ra = fm_from[SA-1:0] + fm_ar[SA-1:0] * fm_from[SA-1+32:32] + fm_at[SA-1:0];\n    if (cv_on) st_ra = cv_st_ra;\n",
            ),
            (
                "    s_issue <= 1'b0; s_wr <= 1'b0; s_len_wr <= 1'b0; st_we <= 1'b0; cb_flush <= 1'b0;\n",
                &format!(
                    "    s_issue <= 1'b0; s_wr <= 1'b0; s_len_wr <= 1'b0; st_we <= 1'b0; cb_flush <= 1'b0; cv_start <= 1'b0;\n    \
                     if (cv_on && cv_st_we) begin st_we <= 1'b1; st_wa <= cv_st_wa; st_wd <= cv_st_wd; st_wm <= {{64{{1'b1}}}}; end\n    \
                     if (cv_flush) cb_flush <= 1'b1;\n    \
                     wd_q <= jw; wd_reg <= 1'b0; wd_prog <= 1'b0;\n    \
                     if ((wd_reg && wd_q[3:0] >= 4'd{v}) || (wd_prog && (wd_q[11:8] >= 4'd{v} || wd_q[15:12] >= 4'd{v} || wd_q[19:16] >= 4'd{v} || wd_q[23:20] >= 4'd{v}))) wide <= 1'b1;\n",
                    v = CONV_PATH_VREGS
                ),
            ),
            (
                "          else if (fm_shape) begin",
                "          else if (cv_shape) begin t_ra <= ld_a[4][8+TA-1:8]; b_ra <= ld_a[0][8+BA-1:8]; state <= CV_T; end\n          else if (fm_shape) begin",
            ),
            (
                "        // A load: its fields and tables;",
                "        // A convolution: its kernel tensor's and its layer's table words read, then every row on the path.\n        \
                 CV_T: state <= CV_GO;\n        CV_GO: begin\n          \
                 cv_kline <= tw[31:0] + (ld_b[4][31:0] >> 4); cv_beats <= cv_c >> 4;\n          \
                 cv_xb <= slott[ld_a[3][12:8]][31:0] + (ld_b[3][31:0] >> 4); cv_xs <= slott[ld_a[3][12:8]][63:32];\n          \
                 cv_ob <= slott[pt_a[0][12:8]][31:0] + (pt_b[0][31:0] >> 4); cv_os <= slott[pt_a[0][12:8]][63:32];\n          \
                 cv_roff <= bw[31:0]; cv_taps <= bw[71:64]; cv_rbytes <= f16 ? bw[63:32] << 1 : bw[63:32] << 2;\n          \
                 cv_start <= 1'b1; state <= CV_RUN;\n        end\n        \
                 CV_RUN: if (!cv_start && !cv_busy) state <= DRAIN;\n        // A load: its fields and tables;",
            ),
            (
                &format!("      if (d_v[{narrow}]) begin q_data[q_wr] <= o_data; q_mask[q_wr] <= o_wmask; q_line[q_wr] <= o_byte[37:6]; q_wr <= q_wr + 4'd1; end\n"),
                &format!(
                    "      if (d_v[{narrow}]) begin q_data[q_wr] <= o_data; q_mask[q_wr] <= o_wmask; q_line[q_wr] <= o_byte[37:6]; q_wr <= q_wr + 4'd1; end\n      \
                     if (cv_q_v) begin q_data[q_wr] <= cv_q_data; q_mask[q_wr] <= cv_q_mask; q_line[q_wr] <= cv_q_line; q_wr <= q_wr + 4'd1; end\n"
                ),
            ),
            (
                &format!("      q_n <= q_n + {{4'd0, d_v[{narrow}]}} - {{4'd0, cb_pop}};\n"),
                &format!("      q_n <= q_n + {{4'd0, d_v[{narrow}]}} + {{4'd0, cv_q_v}} - {{4'd0, cb_pop}};\n"),
            ),
            (
                ", .VREGS(16), .SREGS(8), .FU(FU), .PAIRS(PROWS), .PASSES(PASSES)) vu(",
                &format!(", .VREGS({CONV_PATH_VREGS}), .SREGS(8), .FU(FU), .PAIRS(PROWS), .PASSES(PASSES)) vu("),
            ),
            (
                "ring_open <= 1'b0;\n          state <= HDR0;\n",
                "ring_open <= 1'b0; wide <= 1'b0;\n          state <= HDR0;\n",
            ),
            (
                "              if (dt[0] == 1'b0) ld_a[dt >> 1] <= jw; else ld_b[dt >> 1] <= jw;\n",
                "              if (dt[0] == 1'b0) begin ld_a[dt >> 1] <= jw; wd_reg <= 1'b1; end\n              else ld_b[dt >> 1] <= jw;\n",
            ),
            (
                "              if (dt[0] == 1'b0) pt_a[(dt - {7'd0, nload, 1'b0}) >> 1] <= jw; else pt_b[(dt - {7'd0, nload, 1'b0}) >> 1] <= jw;\n",
                "              if (dt[0] == 1'b0) begin pt_a[(dt - {7'd0, nload, 1'b0}) >> 1] <= jw; wd_reg <= 1'b1; end\n              \
                 else pt_b[(dt - {7'd0, nload, 1'b0}) >> 1] <= jw;\n",
            ),
            (
                "            end else prog[dt - {7'd0, nload, 1'b0} - {7'd0, nput, 1'b0}] <= jw;\n",
                "            end else begin\n              prog[dt - {7'd0, nload, 1'b0} - {7'd0, nput, 1'b0}] <= jw;\n              wd_prog <= 1'b1;\n            end\n",
            ),
            (
                "          else begin li <= 8'd0; gi <= 8'd0; goff <= 16'd0; gk0 <= k; gh0 <= h; state <= LOAD; end\n",
                "          else if (wide) begin fault <= 1'b1; state <= DRAIN; end\n          else begin li <= 8'd0; gi <= 8'd0; goff <= 16'd0; gk0 <= k; gh0 <= h; state <= LOAD; end\n",
            ),
        ],
    )
}

/// `logos_gdn_rows` for an engine of contexts ([`fast_contexts`]): a rows table a context, the context a host write names
/// in its address's top four bits, and the job's rows read from the table of the context it runs for (`ctx`).
fn gdn_rows_contexts(text: &str) -> String {
    replaced(
        text,
        &[
            ("module logos_gdn_rows #(parameter SA = 10, ROW_MAX = 1)", "module logos_gdn_rows #(parameter SA = 10, ROW_MAX = 1, CTX = 1, CB = 1)"),
            ("  (input clk, input rst, input start, input [15:0] rows,", "  (input clk, input rst, input [CB-1:0] ctx, input start, input [15:0] rows,"),
            ("  `LOGOS_RAMSTYLE_ASYNC(ROW_MAX) reg [31:0] rowt [0:ROW_MAX-1];\n", "  localparam TB = $clog2(CTX * ROW_MAX);\n  `LOGOS_RAMSTYLE_ASYNC(CTX*ROW_MAX) reg [31:0] rowt [0:CTX*ROW_MAX-1];\n"),
            ("      3'd1: rowt[host_addr[RB-1:0]] <= host_wdata[31:0];\n", "      3'd1: rowt[host_addr[31:28] * ROW_MAX + host_addr[27:0]] <= host_wdata[31:0];\n"),
            ("  reg [RB-1:0] r_at;\n", "  reg [TB-1:0] r_at;\n"),
            ("      r_at <= {RB{1'b0}};\n", "      r_at <= ctx * ROW_MAX;\n"),
        ],
    )
}

/// `text` with every use of the identifier `from` in its code — never a port's name in a connection, `.from(`, nor a
/// word of a comment — renamed `to`.
fn renamed(text: &str, from: &str, to: &str) -> String {
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$';
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let (code, comment) = line.split_at(line.find("//").unwrap_or(line.len()));
        let bytes = code.as_bytes();
        let mut at = 0;
        while let Some(k) = code[at..].find(from) {
            let start = at + k;
            let end = start + from.len();
            let lone = (start == 0 || !(word(bytes[start - 1]) || bytes[start - 1] == b'.')) && bytes.get(end).is_none_or(|&c| !word(c));
            out.push_str(&code[at..start]);
            out.push_str(if lone { to } else { from });
            at = end;
        }
        out.push_str(&code[at..]);
        out.push_str(comment);
    }
    out
}

/// `text` with every `(from, to)` of `edits` made, each `from` there exactly once.
fn replaced(text: &str, edits: &[(&str, &str)]) -> String {
    let mut out = text.to_string();
    for (from, to) in edits {
        assert_eq!(out.matches(from).count(), 1, "`{from}`");
        out = out.replace(from, to);
    }
    out
}

/// `text` with what runs from `from` up to `to` — each there exactly once, `from` first — replaced by `with`.
fn spliced(text: &str, from: &str, to: &str, with: &str) -> String {
    assert_eq!(text.matches(from).count(), 1, "`{from}`");
    assert_eq!(text.matches(to).count(), 1, "`{to}`");
    let (a, b) = (text.find(from).unwrap(), text.find(to).unwrap());
    assert!(a < b, "`{from}` before `{to}`");
    format!("{}{with}{}", &text[..a], &text[b..])
}

/// Fast clock cycles an engine clock cycle must span at least: the crossing takes the shell's values two cycles after
/// the engine clock's rising edge, the core reads its memories the cycle after the falling edge, and the crossing holds
/// what the engine gives the shell from the cycle after that, all before the next rising edge.
pub const ENGINE_MIN_DIVIDE: u32 = 6;

/// Word `word` of each channel's tile run in the job's runs, `bits` of it, channel 3's highest.
fn runs_field(word: usize, bits: u32) -> String {
    let parts: Vec<String> = (0..4).rev().map(|c| format!("runs[{}:{}]", 256 * c + 32 * word + bits as usize - 1, 256 * c + 32 * word)).collect();
    format!("{{{}}}", parts.join(", "))
}

const ENGINE_VERILOG: &str = r#"// The engine: the sequencer running a step's jobs over the token-row store and the card's memory.
module logos_engine #(parameter ROW_MAX = @ROWS@, VROWS = @VROWS@, STORE_BEATS = @STORE@, JOB_WORDS = @JOBS@,
                        TENSORS = @TENSORS@, LAYERS = @LAYERS@, ROPES = @ROPES@, MAXL = 16, MAXP = 16, MAXI = 64)
  (input clk, input rst,
   input start, input [31:0] job_at, input [31:0] job_end, input [15:0] rows, input state_f16, input cache_int8, input two_words, input per_block,
   output busy, output reg fault, output reg [31:0] pc,
   input host_wr, input [2:0] host_table, input [31:0] host_addr, input [511:0] host_wdata,
   input [31:0] host_raddr, output [511:0] host_rdata,
   output mem_rd, output [31:0] mem_rd_line, input mem_rd_valid, input [511:0] mem_rd_data,
   output mem_wr, output [31:0] mem_wr_line, output [511:0] mem_wr_data, output [63:0] mem_wr_mask, input mem_wr_ready@PORTS@);

  `LOGOS_RAMSTYLE_ASYNC(ROW_MAX) reg [127:0] rowt   [0:ROW_MAX-1];
  `LOGOS_RAMSTYLE_ASYNC(32) reg [63:0]  slott  [0:31];
  reg [63:0]  ropet  [0:ROPES-1];
  // The job words, the tensor and block tables and the token-row store in block RAM, each read where its address is
  // set (`rtl::ram`): the job word at the program counter; a tensor's entry, a load's or the job's; a block's entry, a
  // load's, a store's or the job's layer's; the store's beat the state reads, or the host's while the engine is idle.
  localparam JA = (JOB_WORDS > 1) ? $clog2(JOB_WORDS) : 1, TA = (TENSORS > 1) ? $clog2(TENSORS) : 1,
             BA = (LAYERS > 1) ? $clog2(LAYERS) : 1, SA = (STORE_BEATS > 1) ? $clog2(STORE_BEATS) : 1;
  wire [63:0] jw;
  wire [127:0] tw;
  wire [511:0] bw, st_rd;
  reg [31:0] t_ra, b_ra, st_ra;
  reg st_we;
  reg [31:0] st_wa;
  reg [511:0] st_wd;
  reg [63:0] st_wm;
  reg [6:0] state;
  logos_ram_w64 #(.D(JOB_WORDS), .A(JA)) jobs_ram(.clk(clk), .ra(pc[JA-1:0]), .rd(jw),
    .we(host_wr && host_table == 3'd0), .wa(host_addr[JA-1:0]), .wd(host_wdata[63:0]), .wm(8'hff));
  logos_ram_w128 #(.D(TENSORS), .A(TA)) tens_ram(.clk(clk), .ra(t_ra[TA-1:0]), .rd(tw),
    .we(host_wr && host_table == 3'd3), .wa(host_addr[TA-1:0]), .wd(host_wdata[127:0]), .wm(16'hffff));
  logos_ram_w512 #(.D(LAYERS), .A(BA)) block_ram(.clk(clk), .ra(b_ra[BA-1:0]), .rd(bw),
    .we(host_wr && host_table == 3'd5), .wa(host_addr[BA-1:0]), .wd(host_wdata), .wm({64{1'b1}}));
@STORE_RAM@  assign host_rdata = st_rd;

  // The card's memory: writes queue in block RAM and leave as the card takes them (mem_wr_ready); a read goes out only
  // once every write before it has left, so no read passes a write. A write the full queue cannot take faults the step.
  localparam [5:0] WQ = 6'd32;
  reg rq_v, wq_v;
  reg [31:0] rq_line, wq_line;
  reg [511:0] wq_data;
  reg [63:0] wq_mask;
  reg [5:0] wf_count;
  reg [4:0] wf_head, wf_tail;
  wire [607:0] wf_word;
  logos_ram_w608 #(.D(WQ), .A(5)) write_queue(.clk(clk), .ra(wf_head), .rd(wf_word), .we(wq_v), .wa(wf_tail),
    .wd({wq_mask, wq_line, wq_data}), .wm({76{1'b1}}));
  assign mem_wr = wf_count != 6'd0;
  assign {mem_wr_mask, mem_wr_line, mem_wr_data} = wf_word;
  wire wf_pop = mem_wr && mem_wr_ready;
  // Room for a state's write: the queue less the write in flight and a spare.
  wire wq_room = ({26'd0, wf_count} + {31'd0, wq_v}) < WQ - 2;
  reg rd_pend;
  reg [31:0] rd_pend_line;
  wire wq_empty = (wf_count == 6'd0) && !wq_v;
  assign mem_rd = (rq_v || rd_pend) && wq_empty;
  assign mem_rd_line = rd_pend ? rd_pend_line : rq_line;
  always @(posedge clk) begin
    if (rst) begin
      wf_count <= 6'd0; wf_head <= 5'd0; wf_tail <= 5'd0; rd_pend <= 1'b0;
    end else begin
      if (wq_v) wf_tail <= wf_tail + 5'd1;
      if (wf_pop) wf_head <= wf_head + 5'd1;
      wf_count <= wf_count + {5'd0, wq_v} - {5'd0, wf_pop};
      if (rq_v && !wq_empty) begin rd_pend <= 1'b1; rd_pend_line <= rq_line; end
      else if (mem_rd) rd_pend <= 1'b0;
    end
  end

  // The vector unit and its ports.
  reg s_issue, s_wr, s_len_wr;
  reg [63:0] s_instr;
  reg [3:0] s_wr_reg, s_rd_reg, s_rd_s;
  reg [15:0] s_wr_row, s_rd_row;
  reg [511:0] s_wr_data;
  reg [19:0] s_len_val;
  wire s_busy;
  wire [511:0] s_rd_data;
  wire [31:0] s_rd_s_val;
  logos_simd #(.ROWS(VROWS), .VREGS(16), .SREGS(8), .FU(@FU@)) vu(.clk(clk), .rst(rst), .issue(s_issue), .instr(s_instr), .busy(s_busy),
    .wr(s_wr), .wr_reg(s_wr_reg), .wr_row(s_wr_row), .wr_data(s_wr_data), .len_wr(s_len_wr), .len_val(s_len_val),
    .rd_reg(s_rd_reg), .rd_row(s_rd_row), .rd_data(s_rd_data), .rd_s(s_rd_s), .rd_s_val(s_rd_s_val));

  localparam IDLE = 7'd0, HDR = 7'd1, HEADS = 7'd2, FRESH = 7'd3, DESC = 7'd5, PASS = 7'd6, LOAD = 7'd7,
             LOAD_WAIT = 7'd8, LOAD_LEN = 7'd9, RUN = 7'd10, RUN_GAP = 7'd11, RUN_WAIT = 7'd12, PUT = 7'd13, NEXT = 7'd14,
@MATVEC_STATES@             DL_HDR = 7'd30, DL_ROW = 7'd31, DL_KQ_K = 7'd32, DL_KQ_Q = 7'd33, DL_KQ_L0 = 7'd34, DL_KQ_L1 = 7'd35, DL_KQ_DOT = 7'd36,
             DL_KQ_GAP = 7'd37, DL_KQ_WAIT = 7'd38, DL_E_RD = 7'd39, DL_E_RW = 7'd40, DL_E_OUT = 7'd41, DL_L_OUT = 7'd42,
             DL_CNT = 7'd43, DL_CNT_W = 7'd44, DL_APP = 7'd45, DL_FQ = 7'd46, DL_FZ = 7'd47, DL_FL0 = 7'd48, DL_FL1 = 7'd49,
             DL_FK = 7'd50, DL_FKW = 7'd51, DL_FV = 7'd52, DL_FVW = 7'd53, DL_FDEC = 7'd54, DL_FDECW = 7'd55, DL_FDEC_GAP = 7'd56,
             DL_FDEC_WAIT = 7'd57, DL_FEXP_GAP = 7'd58, DL_FEXP_WAIT = 7'd59, DL_FBETA = 7'd60, DL_FBETAW = 7'd61,
             DL_FB_GAP = 7'd62, DL_FB_WAIT = 7'd63, DL_FL3 = 7'd64, DL_FRUN = 7'd65, DL_FR_GAP = 7'd66, DL_FR_WAIT = 7'd67,
             DL_FLOG = 7'd68, DL_FY = 7'd69, DL_FC = 7'd70, DL_L_RD = 7'd71, DL_L_RW = 7'd72, DL_CNT_WR = 7'd73,
             AT_HDR = 7'd80, AT_ROW = 7'd81, AT_HEAD = 7'd82, AT_Q = 7'd83, AT_Q_GAP = 7'd84, AT_RUN = 7'd85, AT_SCALE = 7'd86,
             AT_SCALE_W = 7'd87, AT_FETCH = 7'd88, AT_FETCH_W = 7'd89, AT_KS = 7'd90, AT_GIVE = 7'd91, AT_TAKEN = 7'd92,
             DL_KQ_BETA = 7'd93;
  // A job done, the sequencer reads the next one's header, until the step's last word. Busy until then and until every
  // write the step made is on the card (the card port takes a write once its channel has written it): the shell takes
  // its channels back once the engine is idle, so a step that ends in writes leaves none of them behind.
  wire [6:0] next_job = (pc < job_end) ? HDR : IDLE;
  assign busy = state != IDLE || !wq_empty;

  // The job's header and descriptors.
  reg [63:0] hdr;
  wire [3:0] unit = hdr[3:0];
  wire heads_first = hdr[4];
  wire [7:0] nfresh = hdr[20:13], nload = hdr[28:21], nput = hdr[36:29];
  wire [15:0] nprog = hdr[52:37];
  reg [31:0] heads;
  `LOGOS_RAMSTYLE_ASYNC(MAXL) reg [63:0] ld_a [0:MAXL-1];
  `LOGOS_RAMSTYLE_ASYNC(MAXL) reg [63:0] ld_b [0:MAXL-1];
  `LOGOS_RAMSTYLE_ASYNC(MAXP) reg [63:0] pt_a [0:MAXP-1];
  `LOGOS_RAMSTYLE_ASYNC(MAXP) reg [63:0] pt_b [0:MAXP-1];
  reg [63:0] prog [0:MAXI-1];
  reg [15:0] dc;


  // The pass: its row and head, the load, store or instruction it is at, and the beat.
  reg [15:0] k;
  reg [31:0] h;
  reg [7:0] li, pi;
  reg [15:0] ii;
  reg [31:0] lb, pb;

  // The row's table entry.
  wire [127:0] rw = rowt[k];
  wire [31:0] region = rw[31:0], position = rw[63:32];
  wire [7:0] ring = rw[71:64];

  // The load at hand.
  wire [63:0] la = ld_a[li], lbw = ld_b[li];
  wire [3:0] l_reg = la[3:0], l_kind = la[7:4];
  wire [15:0] l_what = la[23:8];
  wire [7:0] l_extra = la[31:24];
  wire [31:0] l_len = la[63:32], l_at = lbw[31:0], l_stride = lbw[63:32];
  wire [31:0] l_elem = l_at + h * l_stride;
  wire [31:0] l_beats = (l_len + 32'd15) >> 4;
  wire [63:0] l_slot = slott[l_what[4:0]];
  wire [31:0] l_store_beat = l_slot[31:0] + {16'd0, k} * l_slot[63:32] + (l_elem >> 4) + lb;
  wire [63:0] l_rope = ropet[l_what];
  wire [511:0] l_block = bw;
  // A ring row's index: the row's ring index plus the age, less the ring's rows once it passes them.
  wire [31:0] l_ring_sum = {24'd0, ring} + {24'd0, l_extra};
  wire [31:0] l_ring_idx = (l_ring_sum >= l_block[95:64]) ? l_ring_sum - l_block[95:64] : l_ring_sum;
  wire [31:0] st_bytes = state_f16 ? 32'd2 : 32'd4;
  wire [63:0] l_ring_byte = {32'd0, region} * 64 + l_block[31:0] + l_ring_idx * l_block[63:32] * st_bytes + lb * (state_f16 ? 32 : 64);
  reg [31:0] l_line;
  reg l_half;
  always @* begin
    l_half = 1'b0;
    case (l_kind)
      4'd1: l_line = tw[31:0] + (l_elem >> 4) + lb;
      4'd2: l_line = l_rope[31:0] + (((position * 2 + {31'd0, l_extra[0]}) * l_rope[63:32]) >> 4) + lb;
      default: begin l_line = l_ring_byte[37:6]; l_half = l_ring_byte[5]; end
    endcase
  end
  wire l_from_f16 = (l_kind == 4'd3) && state_f16;
  wire [511:0] widened;
  genvar g;
  generate
    for (g = 0; g < 16; g = g + 1) begin : widen
      logos_f16_to_f32 wv(.h(l_half ? mem_rd_data[256 + 16*g +: 16] : mem_rd_data[16*g +: 16]), .x(widened[32*g +: 32]));
    end
  endgenerate

  // The store at hand.
  wire [63:0] pa = pt_a[pi], pbw = pt_b[pi];
  wire [3:0] p_reg = pa[3:0];
  wire p_scalar = pa[4];
  wire [2:0] p_kind = pa[7:5];
  wire [15:0] p_what = pa[23:8];
  wire p_values = pa[24];
  wire [1:0] p_part = pa[26:25];
  wire [31:0] p_len = pa[63:32], p_at = pbw[31:0], p_stride = pbw[63:32];
  wire [31:0] p_elem = p_at + h * p_stride;
  wire [31:0] p_beats = p_scalar ? 32'd1 : (p_len + 32'd15) >> 4;
  wire [31:0] p_lanes = p_scalar ? 32'd1 : ((p_len - (pb << 4)) >= 32'd16 ? 32'd16 : p_len - (pb << 4));
  wire [63:0] p_slot = slott[p_what[4:0]];
  wire [31:0] p_store_beat = p_slot[31:0] + {16'd0, k} * p_slot[63:32] + (p_elem >> 4) + pb;
  wire [511:0] p_block = bw;
  // The value stored: a vector register's beat, or a scalar register in lane 0.
  wire [511:0] p_data = p_scalar ? {480'd0, s_rd_s_val} : s_rd_data;
  wire [255:0] halves;
  wire [127:0] codes;
  generate
    for (g = 0; g < 16; g = g + 1) begin : narrow
      logos_f32_to_f16 nh(.x(p_data[32*g +: 32]), .h(halves[16*g +: 16]));
      wire signed [31:0] c;
      logos_rne nc(.x(p_data[32*g +: 32]), .n(c));
      assign codes[8*g +: 8] = c[7:0];
    end
  endgenerate
  // A cached row's first byte: its position's row of its head, keys or values.
  wire [31:0] p_cache_off = p_values ? p_block[63:32] : p_block[31:0];
  wire [31:0] p_cache_w = p_values ? p_block[223:192] : p_block[191:160];
  wire [31:0] p_scale_off = p_values ? p_block[127:96] : p_block[95:64];
  wire [31:0] p_cached = position * p_block[159:128] + h;
  reg [63:0] p_byte;
  always @* begin
    case (p_kind)
      3'd1: p_byte = {32'd0, region} * 64 + p_block[31:0] + {24'd0, ring} * p_block[63:32] * st_bytes + pb * (state_f16 ? 32 : 64);
      default: case (p_part)
        2'd0: p_byte = {32'd0, region} * 64 + p_cache_off + p_cached * p_cache_w * 2 + pb * 32;
        2'd1: p_byte = {32'd0, region} * 64 + p_cache_off + p_cached * p_cache_w + pb * 16;
        default: p_byte = {32'd0, region} * 64 + p_scale_off + p_cached * 2;
      endcase
    endcase
  end
  // Lane masks: the lanes below `p_lanes`.
  wire [15:0] lane_mask = (p_lanes >= 32'd16) ? 16'hffff : ((16'd1 << p_lanes[4:0]) - 16'd1);
  wire [5:0] p_off = p_byte[5:0];

@MATVEC_DECLS@
@DELTA_DECLS@
@ATTEND_DECLS@
@QUANTIZER_FEEDS@
  integer i, wi;
  always @(posedge clk) begin
    s_issue <= 1'b0; s_wr <= 1'b0; s_len_wr <= 1'b0; rq_v <= 1'b0; wq_v <= 1'b0;@MATVEC_PULSES@
    @DELTA_PULSES@
    // The host writes the tables and the store between steps.
    if (host_wr) case (host_table)
      3'd1: rowt[host_addr] <= host_wdata[127:0];
      3'd2: slott[host_addr] <= host_wdata[63:0];
      3'd4: ropet[host_addr] <= host_wdata[63:0];
      default: ;
    endcase
    if (rst) begin
      state <= IDLE; fault <= 1'b0;
    end else if (wq_v && wf_count == WQ && !wf_pop) begin
      fault <= 1'b1; state <= IDLE;
    end else case (state)
      IDLE: if (start) begin pc <= job_at; fault <= 1'b0; state <= HDR; end
      HDR: begin
        hdr <= jw; pc <= pc + 32'd1;
        if (jw[3:0] == 4'd2) state <= HEADS;
        else if (jw[3:0] == 4'd1) state <= MV_HDR;
        else if (jw[3:0] == 4'd@DELTA_UNIT@) state <= DL_HDR;
        else if (jw[3:0] == 4'd@ATTEND_UNIT@) state <= AT_HDR;
        // The step's advance moves the host's positions; the card's rows keep theirs to the step's end.
        else if (jw[3:0] == 4'd5) state <= (pc + 32'd1 < job_end) ? HDR : IDLE;
        else begin fault <= 1'b1; state <= IDLE; end
      end
@MATVEC_CASES@@DELTA_CASES@
@ATTEND_CASES@
      HEADS: begin heads <= jw[31:0]; pc <= pc + 32'd1; state <= FRESH; end
      // A fresh slot's width is the vector the job writes; the lanes past it are no vector's, so it needs no clearing.
      FRESH: begin pc <= pc + {24'd0, nfresh}; dc <= 16'd0; state <= DESC; end
      DESC: begin
        // The loads' and stores' words, two each, then the instructions, a word a cycle.
        if (dc == 2 * nload + 2 * nput + nprog) begin k <= 16'd0; h <= 32'd0; state <= PASS; end
        else begin
          if (dc < 2 * nload) begin
            if (dc[0] == 1'b0) ld_a[dc >> 1] <= jw; else ld_b[dc >> 1] <= jw;
          end else if (dc < 2 * nload + 2 * nput) begin
            if (dc[0] == 1'b0) pt_a[(dc - 2 * nload) >> 1] <= jw; else pt_b[(dc - 2 * nload) >> 1] <= jw;
          end else prog[dc - 2 * nload - 2 * nput] <= jw;
          pc <= pc + 32'd1; dc <= dc + 16'd1;
        end
      end
      PASS: begin
        if ((heads_first && h == heads) || (!heads_first && k == rows) || heads == 32'd0 || rows == 16'd0) state <= next_job;
        else begin li <= 8'd0; lb <= 32'd0; state <= LOAD; end
      end
      LOAD: begin
        if (li == nload) begin ii <= 16'd0; state <= RUN; end
        else if (lb == l_beats) state <= LOAD_LEN;
        else if (l_kind == 4'd0) begin
          s_wr <= 1'b1; s_wr_reg <= l_reg; s_wr_row <= lb[15:0]; s_wr_data <= st_rd;
          lb <= lb + 32'd1;
        end else begin
          rq_v <= 1'b1; rq_line <= l_line; state <= LOAD_WAIT;
        end
      end
      LOAD_WAIT: if (mem_rd_valid) begin
        s_wr <= 1'b1; s_wr_reg <= l_reg; s_wr_row <= lb[15:0]; s_wr_data <= l_from_f16 ? widened : mem_rd_data;
        lb <= lb + 32'd1; state <= LOAD;
      end
      LOAD_LEN: begin
        s_len_wr <= 1'b1; s_wr_reg <= l_reg; s_len_val <= l_len[19:0];
        li <= li + 8'd1; lb <= 32'd0; state <= LOAD;
      end
      RUN: begin
        if (ii == nprog) begin pi <= 8'd0; pb <= 32'd0; state <= PUT; end
        else begin s_issue <= 1'b1; s_instr <= prog[ii]; ii <= ii + 16'd1; state <= RUN_GAP; end
      end
      RUN_GAP: state <= RUN_WAIT;
      RUN_WAIT: if (!s_busy) state <= RUN;
      PUT: begin
        // The register's beat is read as the beat is chosen; the write lands a cycle later.
        if (pi == nput) state <= NEXT;
        else if (pb == p_beats) begin pi <= pi + 8'd1; pb <= 32'd0; end
        else if (p_kind == 3'd0 || wq_room) begin
          // A store into the token-row store goes through its write port; one into the card's memory, here.
          if (p_kind != 3'd0) begin
            wq_v <= 1'b1; wq_line <= p_byte[37:6];
            wq_data <= 512'd0; wq_mask <= 64'd0;
            if (p_kind == 3'd1 && !state_f16) begin
              wq_data <= p_data;
              for (i = 0; i < 16; i = i + 1) if (lane_mask[i]) wq_mask[4*i +: 4] <= 4'hf;
            end else if (p_kind == 3'd1 || p_part == 2'd0) begin
              wq_data <= {256'd0, halves} << (8 * p_off);
              for (i = 0; i < 16; i = i + 1) if (lane_mask[i]) wq_mask[p_off + 2*i +: 2] <= 2'b11;
            end else if (p_part == 2'd1) begin
              wq_data <= {384'd0, codes} << (8 * p_off);
              for (i = 0; i < 16; i = i + 1) if (lane_mask[i]) wq_mask[p_off + i] <= 1'b1;
            end else begin
              wq_data <= {496'd0, halves[15:0]} << (8 * p_off);
              wq_mask[p_off +: 2] <= 2'b11;
            end
          end
          pb <= pb + 32'd1;
        end
      end
      NEXT: begin
        if (heads_first) begin
          if (k + 16'd1 == rows) begin k <= 16'd0; h <= h + 32'd1; end else k <= k + 16'd1;
        end else begin
          if (h + 32'd1 == heads) begin h <= 32'd0; k <= k + 16'd1; end else h <= h + 32'd1;
        end
        state <= PASS;
      end
      default: state <= IDLE;
    endcase
  end

  // The store's beat each state reads, the host's while the engine is idle; a load's or the job's tensor; a load's, a
  // store's or the job's layer's block.
  always @* begin
    case (state)
@MATVEC_READS@      LOAD: st_ra = l_store_beat;
@STORE_READS@      default: st_ra = host_raddr;
    endcase
    t_ra = (state == LOAD || state == LOAD_WAIT) ? {16'd0, l_what} : {12'd0, mv_tensor};
    b_ra = (state == LOAD || state == LOAD_WAIT) ? {16'd0, l_what} : (state == PUT) ? {16'd0, p_what} : {24'd0, hdr[12:5]};
  end

@STORE_WRITE_NOTE@  always @* begin
    st_we = 1'b0; st_wa = 32'd0; st_wd = 512'd0; st_wm = 64'd0;
    if (host_wr && host_table >= 3'd6) begin
      st_we = 1'b1; st_wa = host_addr; st_wd = host_wdata; st_wm = {64{1'b1}};
@MATVEC_WRITES@    end else if (!rst && state == PUT && pi != nput && pb != p_beats && p_kind == 3'd0) begin
      st_we = 1'b1; st_wa = p_store_beat;
      if (p_scalar) begin st_wd[32 * p_elem[3:0] +: 32] = s_rd_s_val; st_wm[4 * p_elem[3:0] +: 4] = 4'hf; end
      else for (wi = 0; wi < 16; wi = wi + 1)
        if (lane_mask[wi]) begin st_wd[32*wi +: 32] = s_rd_data[32*wi +: 32]; st_wm[4*wi +: 4] = 4'hf; end
    end
@STORE_WRITES@  end

  // The register a store reads, chosen by the store at hand.
  always @* begin
    s_rd_reg = p_reg; s_rd_row = pb[15:0]; s_rd_s = p_reg;
    @DELTA_READS@
  end
endmodule
"#;

/// How the engine runs its matvec jobs, as fragments its template takes: its ports past the card's memory port, the
/// token-row store's memory, the job's declarations, states, the pulses its states raise for a cycle, its case items,
/// the store beats its states read (case items of the store's read address), the note on the store's write port and
/// its writes into the store (branches of the store's write port).
struct MatvecText {
    ports: String,
    store_ram: String,
    decls: String,
    states: String,
    pulses: String,
    cases: String,
    reads: String,
    write_note: String,
    writes: String,
}

impl MatvecText {
    /// The reference engine's: each job's matrix read a line at a time through the card's memory port into the array's
    /// column of its format, the store in the engine's clock.
    fn reference() -> MatvecText {
        MatvecText {
            ports: String::new(),
            store_ram: REFERENCE_STORE_RAM.to_string(),
            decls: REFERENCE_MATVEC_DECLS.to_string(),
            states: REFERENCE_MATVEC_STATES.to_string(),
            pulses: REFERENCE_MATVEC_PULSES.to_string(),
            cases: REFERENCE_MATVEC_CASES.to_string(),
            reads: REFERENCE_MATVEC_READS.to_string(),
            write_note: REFERENCE_STORE_WRITE_NOTE.to_string(),
            writes: REFERENCE_MATVEC_WRITES.to_string(),
        }
    }
}

impl MatvecText {
    /// The hybrid engine's core ([`hybrid_verilog`]): each job handed to the streaming unit in the fast clock, the store
    /// the fast clock's, both through the crossing between the clocks; with the recurrence unit beside the streaming unit,
    /// a lazily flushed memory's token-row handed to it the same way ([`delta_on_the_fast_side`]). The job's tensor's entry addresses its stream
    /// entry, which the core reads a line at a time through its memory port, and gives the unit with the job's slots,
    /// token-rows and words as one word (`sm_job`, [`HybridJob`]); the unit owns the store and the channels from the edge
    /// the core gives it the job until the unit's busy, as the crossing holds it, falls.
    fn hybrid(s: &Streaming) -> MatvecText {
        let job = HybridJob::BITS;
        MatvecText {
            ports: format!(
                ",\n   output reg sm_own, output reg sm_go, output [{}:0] sm_job, input sm_busy,\n   output [31:0] e_ra, output e_we, \
                 output [31:0] e_wa, output [511:0] e_wd, output [63:0] e_wm, input [511:0] e_rd{}{}",
                job - 1,
                if s.recurrence {
                    format!(",\n   output reg sm_gdn_go, output [{}:0] sm_gdn_job, input sm_gdn_fault", GdnJob::bits(s) - 1)
                } else {
                    String::new()
                },
                if s.vectors {
                    format!(",\n   output reg sm_vec_go, output [{}:0] sm_vec_job, input sm_vec_fault", VectorSideJob::BITS - 1)
                } else {
                    String::new()
                }
            ) + &if s.attention { format!(",\n   output reg sm_att_go, output [{}:0] sm_att_job", AttendJob::BITS - 1) } else { String::new() },
            store_ram: String::new(),
            decls: format!(
                r#"  // The matvec job, handed to the streaming unit in the fast clock: the stream entry the job's tensor's entry addresses,
  // with the job's slots, token-rows and words.
  reg [19:0] mv_tensor;
  reg [4:0] mv_from, mv_to;
  wire [127:0] mv_entry = tw;
  reg [511:0] se0, se1, se2;
  reg [1:0] se_i;
  assign sm_job = {{two_words, rows[7:0], slott[mv_to], slott[mv_from], se2, se1, se0[{entry_top}:0]}};
  // The store, the fast clock's through the crossing.
  assign st_rd = e_rd;
  assign e_ra = st_ra;
  assign e_we = st_we;
  assign e_wa = st_wa;
  assign e_wd = st_wd;
  assign e_wm = st_wm;
{vector_job}@QUANTIZERS@"#,
                entry_top = HybridJob::ENTRY - 1,
                vector_job = if s.vectors { VECTOR_JOB } else { "" },
            ),
            states: format!(
                "             MV_HDR = 7'd15, MV_SL = 7'd16, MV_SLW = 7'd17, MV_RUN = 7'd18,{}\n",
                if s.vectors { " V_SG = 7'd76, V_SIDE = 7'd77," } else { "" }
            ),
            pulses: format!(" sm_go <= 1'b0; if (rst) sm_own <= 1'b0;{}", if s.vectors { " sm_vec_go <= 1'b0;" } else { "" }),
            cases: format!(
                r#"      MV_HDR: begin
        mv_tensor <= jw[19:0]; mv_from <= jw[24:20]; mv_to <= jw[29:25]; pc <= pc + 32'd1; se_i <= 2'd0;
        if (!per_block{one_word}) begin fault <= 1'b1; state <= IDLE; end else state <= MV_SL;
      end
      // The stream entry a line a read, then the store and the channels the unit's, and its start.
      MV_SL: begin
        if (se_i == 2'd3) begin sm_own <= 1'b1; sm_go <= 1'b1; state <= MV_RUN; end
        else begin rq_v <= 1'b1; rq_line <= mv_entry[31:0] + {{30'd0, se_i}}; state <= MV_SLW; end
      end
      MV_SLW: if (mem_rd_valid) begin
        case (se_i)
          2'd0: se0 <= mem_rd_data;
          2'd1: se1 <= mem_rd_data;
          default: se2 <= mem_rd_data;
        endcase
        se_i <= se_i + 2'd1; state <= MV_SL;
      end
      MV_RUN: if (!sm_go && !sm_busy) begin sm_own <= 1'b0; state <= next_job; end
{vector_cases}"#,
                one_word = if s.set.words == Words::Two { "" } else { " || two_words" },
                vector_cases = if s.vectors { VECTOR_CASES } else { "" },
            ),
            reads: String::new(),
            write_note: "  // The store's write port: the host's beat; a store's lanes; the units' writes.\n".to_string(),
            writes: String::new(),
        }
    }
}

/// The word a hybrid engine's core gives its streaming unit with a matvec job, its fields from the lowest bit: the
/// stream entry's first line's seven words ([`super::streaming::StreamEntry`]: the seventh its greedy flag and, from bit
/// 1, its fold), its four tile runs (its other two lines), the input slot's and the output slot's table entries, the
/// step's token-rows and whether it runs two words.
pub struct HybridJob;

impl HybridJob {
    pub const GREEDY: usize = 192;
    pub const FOLD: usize = Self::GREEDY + 1;
    pub const ENTRY: usize = 224;
    pub const RUNS: usize = Self::ENTRY;
    pub const FROM: usize = Self::RUNS + 1024;
    pub const TO: usize = Self::FROM + 64;
    pub const TOKENS: usize = Self::TO + 64;
    pub const TWO: usize = Self::TOKENS + 8;
    pub const BITS: usize = Self::TWO + 1;
}

/// The word a hybrid engine's core gives its attention side with a token-row of an attention job
/// ([`Streaming::attention`]), its fields from the lowest bit, each 32 bits but the last: the row's region (its first
/// line), its block's keys', values', key scales' and value scales' offsets in it, the cached heads, the row's positions
/// (its own and every one before), the score scale's bits, the store beats of the row's query and of its output, and
/// the query heads (16 bits).
pub struct AttendJob;

impl AttendJob {
    pub const REGION: usize = 0;
    pub const KEYS: usize = 32;
    pub const VALUES: usize = 64;
    pub const KEY_SCALES: usize = 96;
    pub const VALUE_SCALES: usize = 128;
    pub const KV_HEADS: usize = 160;
    pub const POSITIONS: usize = 192;
    pub const SCALE: usize = 224;
    pub const QUERY: usize = 256;
    pub const OUTPUT: usize = 288;
    pub const HEADS: usize = 320;
    pub const BITS: usize = 336;
}

/// The word a hybrid engine's core gives its vector unit's side with a vector job ([`Streaming::vectors`]), its fields
/// from the lowest bit: the place of the job's header in the job words, the step's token-rows (16 bits), and whether the
/// state stores binary16.
pub struct VectorSideJob;

impl VectorSideJob {
    pub const PC: usize = 0;
    pub const ROWS: usize = 32;
    pub const F16: usize = 48;
    pub const BITS: usize = 49;
}

/// The core of a hybrid engine whose vector jobs run on the fast side ([`Streaming::vectors`]): the engine's template with a
/// vector job's header going to the hand-off ([`VECTOR_CASES`]) and the core's own vector-job states, their descriptors,
/// their loads' and stores' places and their store writes gone; the vector unit stays for the recurrence unit's `k·q`
/// of a memory rewritten every token, and with `handed_off` (every memory lazily flushed, its token-rows the recurrence
/// unit's) it goes too: its registers, its instance, its pulses and its read port.
fn vector_jobs_on_the_fast_side(template: &str, handed_off: bool) -> String {
    let text = replaced(
        template,
        &[
            ("        if (jw[3:0] == 4'd2) state <= HEADS;\n", "        if (jw[3:0] == 4'd2) state <= V_SG;\n"),
            (
                "  reg [31:0] heads;\n  `LOGOS_RAMSTYLE_ASYNC(MAXL) reg [63:0] ld_a [0:MAXL-1];\n  `LOGOS_RAMSTYLE_ASYNC(MAXL) reg [63:0] ld_b [0:MAXL-1];\n  `LOGOS_RAMSTYLE_ASYNC(MAXP) reg [63:0] pt_a [0:MAXP-1];\n  `LOGOS_RAMSTYLE_ASYNC(MAXP) reg [63:0] pt_b [0:MAXP-1];\n  reg [63:0] prog [0:MAXI-1];\n  reg [15:0] dc;\n",
                "",
            ),
            (
                "  // The pass: its row and head, the load, store or instruction it is at, and the beat.\n  reg [15:0] k;\n  reg [31:0] h;\n  \
                 reg [7:0] li, pi;\n  reg [15:0] ii;\n  reg [31:0] lb, pb;\n",
                "  // The row and the head a unit's job is at.\n  reg [15:0] k;\n  reg [31:0] h;\n",
            ),
            ("  wire [7:0] ring = rw[71:64];\n", ""),
            ("@MATVEC_READS@      LOAD: st_ra = l_store_beat;\n", "@MATVEC_READS@"),
            (
                "    t_ra = (state == LOAD || state == LOAD_WAIT) ? {16'd0, l_what} : {12'd0, mv_tensor};\n    \
                 b_ra = (state == LOAD || state == LOAD_WAIT) ? {16'd0, l_what} : (state == PUT) ? {16'd0, p_what} : {24'd0, hdr[12:5]};\n",
                "    t_ra = {12'd0, mv_tensor};\n    b_ra = {24'd0, hdr[12:5]};\n",
            ),
            ("    s_rd_reg = p_reg; s_rd_row = pb[15:0]; s_rd_s = p_reg;\n", "    s_rd_reg = 4'd0; s_rd_row = 16'd0; s_rd_s = 4'd0;\n"),
        ],
    );
    let text = spliced(&text, "  // The load at hand.\n", "@MATVEC_DECLS@", "  wire [31:0] st_bytes = state_f16 ? 32'd2 : 32'd4;\n  genvar g;\n\n");
    let text = spliced(&text, "      HEADS: begin heads <= jw[31:0];", "      default: state <= IDLE;\n    endcase", "");
    let text = spliced(&text, "    end else if (!rst && state == PUT && pi != nput", "@STORE_WRITES@", "    end\n");
    for gone in [
        "ld_a", "pt_b", "prog[", "l_store_beat", "p_store_beat", "l_ring_byte", "p_cached", "lane_mask", "p_byte", "l_line", "HEADS:",
        "LOAD_WAIT:", "PUT:", "NEXT:", "state <= HEADS",
    ] {
        assert!(!text.contains(gone), "a core whose vector jobs run on the fast side keeps no `{gone}`");
    }
    if !handed_off {
        return text;
    }
    let text = spliced(&text, "  // The vector unit and its ports.\n", "  localparam IDLE = 7'd0,", "");
    let text = replaced(
        &text,
        &[
            ("    s_issue <= 1'b0; s_wr <= 1'b0; s_len_wr <= 1'b0; rq_v <= 1'b0; wq_v <= 1'b0;", "    rq_v <= 1'b0; wq_v <= 1'b0;"),
            (
                "  // The register a store reads, chosen by the store at hand.\n  always @* begin\n    s_rd_reg = 4'd0; s_rd_row = 16'd0; s_rd_s = 4'd0;\n    @DELTA_READS@\n  end\n",
                "",
            ),
        ],
    );
    for gone in ["logos_simd", "s_issue", "s_rd_reg", "s_wr_data", "s_busy"] {
        assert!(!text.contains(gone), "a core that hands every token-row and vector job to the fast side keeps no `{gone}`");
    }
    text
}

/// The recurrence unit of a vectored hybrid engine's core whose memories are all lazily flushed ([`Streaming::vectors`]
/// with a log of more than one token): the job's slots and its block's places read, and each token-row handed whole to
/// the fast side's recurrence unit ([`GdnJob`], [`GDN_CASES`]); a block rewritten every token (a log of none) faults, the
/// row engine that would update it absent with the vector unit its `k·q` needs.
fn delta_handed_off(hd: usize, t_max: usize, normed: bool) -> UnitVerilog {
    let decls = format!(
        r#"  // The recurrence unit: the job's slots and its block's place in a region, each token-row the fast side's.
  localparam HD = {hd}, TMAX = {t_max};
  localparam [6:0] DL_SG = 7'd74, DL_SIDE = 7'd75;
  reg [4:0] dl_qs, dl_ks, dl_vs, dl_lds, dl_dcs, dl_gts, dl_tos;
  reg [19:0] dl_vat;
  wire [511:0] dlb = bw;
  wire [31:0] dl_mem = dlb[127:96], dl_log = dlb[159:128], dl_entry = dlb[191:160];
  wire [31:0] dl_nk = dlb[223:192], dl_nv = dlb[255:224], dl_hd = dlb[287:256];
  wire [31:0] dl_fval = dlb[319:288], dl_fdec = dlb[351:320], dl_fgate = dlb[383:352], dl_T = dlb[415:384], dl_scale = dlb[447:416];
  // A slot's beat holding element `e` of the row at hand; the element is the beat's lane `e mod 16`.
  function [31:0] element_beat(input [63:0] slot, input [31:0] e);
    element_beat = slot[31:0] + {{16'd0, k}} * slot[63:32] + (e >> 4);
  endfunction
{GDN_JOB}"#
    );
    let cases = format!(
        r#"      DL_HDR: begin
        dl_qs <= jw[4:0]; dl_ks <= jw[9:5]; dl_vs <= jw[14:10]; dl_lds <= jw[19:15];
        dl_dcs <= jw[24:20]; dl_gts <= jw[29:25]; dl_tos <= jw[34:30]; dl_vat <= jw[54:35];
        pc <= pc + 32'd1; k <= 16'd0; state <= DL_ROW;
      end
      DL_ROW: begin
        if (k == rows) state <= next_job;
        else if (dl_T == 32'd0) begin fault <= 1'b1; state <= IDLE; end
        else state <= DL_SG;
      end
{GDN_CASES}"#
    );
    let unit = UnitVerilog { decls, cases, reads: String::new(), writes: String::new() };
    let unit = if normed { delta_normed_core(unit) } else { unit };
    for gone in ["dre", "dlr", "dl_kv", "dl_qv", "DL_KQ", "DL_E_", "dl_vals", "s_issue", "s_wr"] {
        for text in [&unit.decls, &unit.cases] {
            assert!(!text.contains(gone), "a recurrence unit that hands every token-row to the fast side keeps no `{gone}`");
        }
    }
    unit
}

/// The attention unit of a hybrid engine whose attention runs on the fast side ([`Streaming::attention`]): each
/// token-row of an attention job handed whole to the attention side ([`AttendJob`]) once every write before it has
/// left, the core waiting for it; no attention unit, no quantizer, no cache read of the core's own.
fn attend_handed_off() -> UnitVerilog {
    let decls = r#"  // The attention unit: the job's slots, heads and score scale, its block's cache in a region, each token-row the
  // fast side's.
  localparam [6:0] AT_SG = 7'd94, AT_SIDE = 7'd95;
  reg [4:0] at_qs, at_tos;
  reg [15:0] at_heads;
  reg [31:0] at_scale;
  wire [511:0] atb = bw;
  wire [31:0] at_keys = atb[31:0], at_vals = atb[63:32], at_kscales = atb[95:64], at_vscales = atb[127:96];
  wire [31:0] at_kvh = atb[159:128];
  wire [63:0] at_qslot = slott[at_qs], at_oslot = slott[at_tos];
  wire [31:0] at_qbeat = at_qslot[31:0] + {16'd0, k} * at_qslot[63:32], at_obeat = at_oslot[31:0] + {16'd0, k} * at_oslot[63:32];
  assign sm_att_job = {at_heads, at_obeat, at_qbeat, at_scale, position + 32'd1, at_kvh, at_vscales, at_kscales, at_vals, at_keys,
    region};
"#
    .to_string();
    let cases = r#"      AT_HDR: begin
        at_qs <= jw[4:0]; at_tos <= jw[9:5]; at_heads <= jw[25:10]; at_scale <= jw[57:26];
        pc <= pc + 32'd1; k <= 16'd0; state <= AT_ROW;
      end
      AT_ROW: if (k == rows) state <= next_job; else state <= AT_SG;
      // A token-row on the fast side's attention side, once every write before it has left.
      AT_SG: if (wq_empty) begin sm_own <= 1'b1; sm_att_go <= 1'b1; state <= AT_SIDE; end
      AT_SIDE: if (!sm_att_go && !sm_busy) begin sm_own <= 1'b0; k <= k + 16'd1; state <= AT_ROW; end
"#
    .to_string();
    UnitVerilog { decls, cases, reads: String::new(), writes: String::new() }
}

/// The core's vector job for the fast side ([`VectorSideJob`]).
const VECTOR_JOB: &str = r#"  // A vector job as the fast side's vector unit takes it (`VectorSideJob`): its header's place, the step's token-rows,
  // whether the state stores binary16.
  reg [31:0] vs_pc;
  assign sm_vec_job = {state_f16, rows, vs_pc};
"#;

/// The core's hand-off of a vector job to the fast side and its taking the store and the channels back, the program
/// counter past the job's words.
const VECTOR_CASES: &str = r#"      // A vector job on the fast side's vector unit — its loads, its micro-program and its stores, each pass of a row and a
      // head — once every write before it has left; the job's words passed.
      V_SG: if (wq_empty) begin
        vs_pc <= pc - 32'd1;
        pc <= pc + 32'd1 + {24'd0, nfresh} + {23'd0, nload, 1'b0} + {23'd0, nput, 1'b0} + {16'd0, nprog};
        sm_own <= 1'b1; sm_vec_go <= 1'b1; state <= V_SIDE;
      end
      V_SIDE: if (!sm_vec_go && !sm_busy) begin
        sm_own <= 1'b0;
        if (sm_vec_fault) begin fault <= 1'b1; state <= IDLE; end else state <= next_job;
      end
"#;

/// The word a hybrid engine's core gives its recurrence unit with a job of a lazily flushed memory's token-rows
/// ([`Streaming::recurrence`]), its fields from the lowest bit: the job's rows (16 bits); its block's memory's and log's
/// offsets in a row's region and a log entry's bytes, its key and value heads, a log entry's values', decay exponents' and
/// gates' first elements, the read-out's scale's bits (32 bits each); the store slots of the rows' query, output, key
/// heads, value heads, decay exponents and gates (5 bits each) and the values' first element in theirs (20 bits); the
/// tokens a flush takes (8 bits), and whether the state stores binary16. The fast side lays each row's region and store
/// beats from its copies of the rows and slots tables (`logos_gdn_rows`).
pub struct GdnJob;

impl GdnJob {
    pub const ROWS: usize = 0;
    pub const MEMORY: usize = 16;
    pub const LOG: usize = 48;
    pub const ENTRY: usize = 80;
    pub const KEY_HEADS: usize = 112;
    pub const VALUE_HEADS: usize = 144;
    pub const VALUES: usize = 176;
    pub const DECAYS: usize = 208;
    pub const GATES: usize = 240;
    pub const SCALE: usize = 272;
    pub const QUERY: usize = 304;
    pub const OUTPUT: usize = 309;
    pub const KEY: usize = 314;
    pub const VALUE: usize = 319;
    pub const LOG_DECAY: usize = 324;
    pub const GATE: usize = 329;
    pub const VALUE_AT: usize = 334;
    pub const TOKENS: usize = 354;
    pub const F16: usize = 362;
    pub const BITS: usize = 363;
    /// A build whose recurrence makes its gated norms ([`Streaming::delta_norm`]): whether the job carries one, its
    /// gate's slot, the norm's epsilon and its weight's first line, past the fields every build takes.
    pub const NORM: usize = 363;
    pub const Z: usize = 364;
    pub const EPS: usize = 369;
    pub const W_LINE: usize = 401;
    pub const NORMED_BITS: usize = 433;

    /// The bits of the job word of the build `s`.
    pub fn bits(s: &Streaming) -> usize {
        if s.delta_norm { GdnJob::NORMED_BITS } else { GdnJob::BITS }
    }
}

/// The recurrence unit of a hybrid engine whose lazily flushed memories run on the fast side
/// ([`Streaming::recurrence`]): `core`'s ([`delta_verilog`]'s) with every token's update kept and a lazily flushed
/// memory's token-row handed whole to the fast side's recurrence unit ([`GdnJob`]) — the log's count read, the row's
/// entry appended, the fold, the read-out, the flush and the count written back there — once every write before it has
/// left; the core's lazy row engine, its log's append and count, and their registers and states gone.
fn delta_on_the_fast_side(core: UnitVerilog) -> UnitVerilog {
    let decls = replaced(
        &core.decls,
        &[
            ("  reg [19:0] dl_vat;\n", "  reg [19:0] dl_vat;\n  localparam [6:0] DL_SG = 7'd74, DL_SIDE = 7'd75;\n"),
            (
                "  reg [31:0] dg, dr, dj, de, dl_count, dob;\n  reg [1:0] dfld;\n  reg [2:0] dl_ii;\n  reg dl_flush;\n",
                "  reg [31:0] dg, dr, dj, dob;\n",
            ),
            (
                "  reg [32*HD-1:0] dl_kv, dl_qv, dl_y, dl_c;\n  `LOGOS_RAMSTYLE_ASYNC(TMAX) reg [32*HD-1:0] dl_vals [0:TMAX-1];\n  \
                 reg [31:0] dl_kq, dl_decay, dl_beta, dl_dec_e, dl_beta_e;\n  reg [3:0] dl_rd_reg, dl_rd_s;\n  reg [15:0] dl_rd_row;\n  // The vector unit's read port is the fold's while its folded query and carried values are read out: a function of\n  // the state, so no power-up value of a register of its own can hand it to the fold outside those states.\n  wire dl_rd = state == DL_FY || state == DL_FC;\n",
                "  reg [32*HD-1:0] dl_kv, dl_qv;\n  reg [31:0] dl_kq, dl_decay, dl_beta;\n",
            ),
            ("  wire [255:0] dl_halves, dl_app_halves;\n", "  wire [255:0] dl_halves;\n"),
            ("      logos_f32_to_f16 na(.x(dl_app_beat[32*dgv +: 32]), .h(dl_app_halves[16*dgv +: 16]));\n", ""),
            (
                "  // The row engines: every token's update, and a lazily flushed memory's read-out and replay.\n  \
                 reg dre_v, dre_first, dlr_v, dlr_first, dlr_log_wr;\n  reg [511:0] dre_data;\n  wire dre_ov, dre_ol, dlr_ov, dlr_ol, dlr_busy;\n  \
                 wire [511:0] dre_od, dlr_od;\n  wire [31:0] dre_or, dlr_or;\n",
                "  // The row engine: every token's update.\n  reg dre_v, dre_first;\n  reg [511:0] dre_data;\n  wire dre_ov, dre_ol;\n  \
                 wire [511:0] dre_od;\n  wire [31:0] dre_or;\n",
            ),
            (
                "  wire dl_out_v = dl_lazy ? dlr_ov : dre_ov, dl_out_l = dl_lazy ? dlr_ol : dre_ol;\n  \
                 assign dl_out_beat_data = dl_lazy ? dlr_od : dre_od;\n  wire [31:0] dl_out_read = dl_lazy ? dlr_or : dre_or;\n",
                "  wire dl_out_v = dre_ov, dl_out_l = dre_ol;\n  assign dl_out_beat_data = dre_od;\n  wire [31:0] dl_out_read = dre_or;\n",
            ),
        ],
    );
    let decls = spliced(&decls, "  // The log: its count's line, the entry at hand, a field's beat.\n", "  // A state beat as the engine reads it:", "");
    let decls = spliced(&decls, "  // An entry's key or value beat at hand during the fold", "  // The row engine: every token's update.\n", "");
    let decls = spliced(&decls, "  logos_gdn_lazy_row #(", "  wire dl_lazy = dl_T != 32'd0;\n", "");
    let decls = spliced(&decls, "  // A fold's instruction.\n", "  // The read-out's element of the job's output.\n", GDN_JOB);
    let cases = replaced(
        &core.cases,
        &[
            (
                "        else begin h <= 32'd0; dg <= 32'd0; dj <= 32'd0; state <= dl_lazy ? DL_CNT : DL_KQ_K; end\n",
                "        else begin h <= 32'd0; dg <= 32'd0; dj <= 32'd0; state <= dl_lazy ? DL_SG : DL_KQ_K; end\n",
            ),
            (
                "      DL_E_OUT, DL_L_OUT: begin\n        if (dl_out_v && !(dl_lazy && !dl_flush)) begin\n",
                "      DL_E_OUT: begin\n        if (dl_out_v) begin\n",
            ),
            (
                "            if (h + 32'd1 == dl_nv) begin\n              if (dl_lazy) state <= DL_CNT_WR;\n              \
                 else begin k <= k + 16'd1; state <= DL_ROW; end\n            end else begin\n",
                "            if (h + 32'd1 == dl_nv) begin k <= k + 16'd1; state <= DL_ROW; end\n            else begin\n",
            ),
            ("              state <= dl_lazy ? DL_FQ : DL_KQ_K;\n", "              state <= DL_KQ_K;\n"),
            (
                "          end else begin dr <= dr + 32'd1; state <= dl_lazy ? DL_L_RD : DL_E_RD; end\n",
                "          end else begin dr <= dr + 32'd1; state <= DL_E_RD; end\n",
            ),
        ],
    );
    let lazy = cases.find("      // A lazily flushed memory: the row's token logged after the count,").expect("the lazily flushed memory's states");
    assert!(cases[lazy..].trim_end().ends_with("end") && cases[lazy..].matches("DL_CNT_WR: if (wq_room) begin").count() == 1, "the lazy states end the unit's cases");
    let cases = format!("{}{GDN_CASES}", &cases[..lazy]);
    let reads = replaced(
        &core.reads,
        &[("      DL_KQ_Q, DL_FQ: st_ra = dl_qs_beat;\n", "      DL_KQ_Q: st_ra = dl_qs_beat;\n"), ("      DL_APP: st_ra = dl_app_src;\n", "")],
    );
    let writes = replaced(
        &core.writes,
        &[("    else if (!rst && (state == DL_E_OUT || state == DL_L_OUT) && dl_out_l) begin\n", "    else if (!rst && state == DL_E_OUT && dl_out_l) begin\n")],
    );
    let unit = UnitVerilog { decls, cases, reads, writes };
    for gone in [
        "dlr", "dl_vals", "dl_y", "dl_c[", "dl_ii", "dl_rd", "dl_word", "DL_F", "DL_L_", "DL_CNT", "DL_APP", "dl_app", "dl_count", "dfld",
        "dl_flush", "dl_log_byte", "dl_entry_byte",
    ] {
        for text in [&unit.decls, &unit.cases, &unit.reads, &unit.writes] {
            assert!(!text.contains(gone), "the fast side's recurrence leaves the core no `{gone}`");
        }
    }
    unit
}

/// [`delta_handed_off`]'s unit for a build whose recurrence makes its gated norms ([`Streaming::delta_norm`]): a job that
/// carries one (its slots word's bit 55, its gate's slot in bits 56 to 60) reads its next word — the norm weight's
/// tensor and the norm's epsilon — and a step later the weight's first line from the tensor table, as a matrix job reads
/// its entry, and hands the fast side all four with the job ([`GdnJob::NORM`] on).
fn delta_normed_core(unit: UnitVerilog) -> UnitVerilog {
    let decls = replaced(
        &unit.decls,
        &[
            (
                "  localparam [6:0] DL_SG = 7'd74, DL_SIDE = 7'd75;\n",
                "  localparam [6:0] DL_SG = 7'd74, DL_SIDE = 7'd75, DL_NORM = 7'd78, DL_WLINE = 7'd79;\n  \
                 // The job's gated norm: whether it carries one, its gate's slot, its epsilon, its weight's first line.\n  \
                 reg dl_norm;\n  reg [4:0] dl_zs;\n  reg [31:0] dl_eps, dl_wline;\n",
            ),
            ("  assign sm_gdn_job = {state_f16,", "  assign sm_gdn_job = {dl_wline, dl_eps, dl_zs, dl_norm, state_f16,"),
        ],
    );
    let cases = replaced(
        &unit.cases,
        &[(
            "        pc <= pc + 32'd1; k <= 16'd0; state <= DL_ROW;\n      end\n",
            "        dl_norm <= jw[55]; dl_zs <= jw[60:56];\n        pc <= pc + 32'd1; k <= 16'd0; state <= jw[55] ? DL_NORM : DL_ROW;\n      end\n      \
             // A gated norm's word: its weight's tensor, whose first line the tensor table gives a step later, and its\n      \
             // epsilon.\n      \
             DL_NORM: begin mv_tensor <= jw[19:0]; dl_eps <= jw[51:20]; pc <= pc + 32'd1; state <= DL_WLINE; end\n      \
             DL_WLINE: begin dl_wline <= mv_entry[31:0]; state <= DL_ROW; end\n",
        )],
    );
    UnitVerilog { decls, cases, ..unit }
}

/// The core's recurrence job for the fast side ([`GdnJob`]).
const GDN_JOB: &str = r#"  // A lazily flushed memory's token-rows as the fast side's recurrence unit takes them (`GdnJob`): the step's rows, the
  // block's places, the read-out's scale, the slots of the rows' query, output, key heads, value heads, decay exponents
  // and gates and the values' first element, the tokens a flush takes, whether the state stores binary16.
  assign sm_gdn_job = {state_f16, dl_T[7:0], dl_vat, dl_gts, dl_lds, dl_vs, dl_ks, dl_tos, dl_qs, dl_scale, dl_fgate, dl_fdec,
    dl_fval, dl_nv, dl_nk, dl_entry, dl_log, dl_mem, rows};
"#;

/// The core's hand-off of a lazily flushed memory's token-rows to the fast side and its taking the store and the
/// channels back: a block whose heads are not the unit's width, or whose log holds more tokens than the unit's, faults.
const GDN_CASES: &str = r#"      // A lazily flushed memory's token-rows on the fast side's recurrence unit, all of the job's at once — each row's token
      // logged after its log's count, the query folded back through the log for each head, each value row read through the
      // folded query and, once the log is full, replayed and stored, the count written back — once every write before it
      // has left.
      DL_SG: if (wq_empty) begin
        if (dl_hd != $unsigned(HD) || dl_T > $unsigned(TMAX)) begin fault <= 1'b1; state <= IDLE; end
        else begin sm_own <= 1'b1; sm_gdn_go <= 1'b1; state <= DL_SIDE; end
      end
      DL_SIDE: if (!sm_gdn_go && !sm_busy) begin
        sm_own <= 1'b0;
        if (sm_gdn_fault) begin fault <= 1'b1; state <= IDLE; end else begin k <= rows; state <= DL_ROW; end
      end
"#;

const REFERENCE_STORE_RAM: &str = r#"  logos_ram_w512 #(.D(STORE_BEATS), .A(SA)) store_ram(.clk(clk), .ra(st_ra[SA-1:0]), .rd(st_rd),
    .we(st_we), .wa(st_wa[SA-1:0]), .wd(st_wd), .wm(st_wm));
"#;

const REFERENCE_MATVEC_DECLS: &str = r#"  // The array's job: its tensor's format, rows and blocks, the token-rows' activations a weight block at a time, and
  // the weight stream two rows' blocks a beat, each block gathered from the lines it spans.
  reg [19:0] mv_tensor;
  reg [4:0] mv_from, mv_to;
  wire [127:0] mv_entry = tw;
  wire [7:0] mv_type = mv_entry[39:32];
  wire [31:0] mv_in = {8'd0, mv_entry[63:40]}, mv_out = {8'd0, mv_entry[87:64]};
  reg mv_known;
  reg [3:0] mv_fmt;
  reg [15:0] mv_blen, mv_bbytes;
  always @* begin
    mv_known = 1'b0; mv_fmt = 4'd0; mv_blen = 16'd32; mv_bbytes = 16'd0;
    case (mv_type)
@FORMAT_CASES@      default: ;
    endcase
  end
  wire [31:0] mv_blocks = mv_in / {16'd0, mv_blen};
  wire [31:0] mv_pairs = (mv_out + 32'd1) >> 1;
  wire [31:0] mv_row_bytes = mv_blocks * {16'd0, mv_bbytes};
  reg mv_start, act_wr;
  wire w_valid = (state == MV_PRESENT);
  reg [7:0] act_t;
  reg [15:0] act_b;
  reg [31:0] mv_p, mv_b, aj;
@QUANTIZERS@
  reg [@WIDEST@:0] w_blk1, w_blk2;
  reg [2559:0] gbuf;
  reg [63:0] g_byte;
  reg [3:0] g_i, g_n;
  reg g_two;
  // The lines a block of the row at hand spans from its byte in the line.
  wire [15:0] g_lines = (g_byte[5:0] + mv_bbytes + 16'd63) >> 6;
@COLUMNS@
  wire mv_ready = @READY@;
  wire mv_col_busy = @COLUMN_BUSY@;
  reg o_v;
  reg [15:0] o_p;
  reg [7:0] o_t;
  reg [31:0] o_1, o_2;
  always @* begin
    o_v = 1'b0; o_p = 16'd0; o_t = 8'd0; o_1 = 32'd0; o_2 = 32'd0;
@OUT_MUX@  end
  wire [63:0] mv_to_slot = slott[mv_to];
  wire [31:0] o_e = {15'd0, o_p, 1'b0};
  wire [31:0] o_beat = mv_to_slot[31:0] + {24'd0, o_t} * mv_to_slot[63:32] + (o_e >> 4);
  wire [63:0] mv_from_slot = slott[mv_from];
  // A gathered block: the lines it spans, from its first byte.
  wire [2559:0] g_shifted = gbuf >> (8 * g_byte[5:0]);
  wire [63:0] g_row_byte = {32'd0, mv_entry[31:0]} * 64 + mv_b * mv_bbytes;
"#;

const REFERENCE_MATVEC_STATES: &str = r#"             MV_HDR = 7'd15, MV_ACT = 7'd16, MV_ACT_WR = 7'd17, MV_ACT_GAP = 7'd18, MV_GO = 7'd19, MV_NEXT = 7'd20,
             MV_G = 7'd21, MV_GW = 7'd22, MV_PRESENT = 7'd23, MV_DRAIN = 7'd24,
"#;

const REFERENCE_MATVEC_CASES: &str = r#"      MV_HDR: begin
        mv_tensor <= jw[19:0]; mv_from <= jw[24:20]; mv_to <= jw[29:25]; pc <= pc + 32'd1;
        act_t <= 8'd0; act_b <= 16'd0; aj <= 32'd0;
        if (!per_block) begin fault <= 1'b1; state <= IDLE; end else state <= MV_ACT;
      end
      MV_ACT: begin
        // A token-row's weight block of activations, a beat a cycle into the quantizer, then its codes and scale into
        // the column.
        if (!mv_known) begin fault <= 1'b1; state <= IDLE; end
        else if ({8'd0, act_t} == {8'd0, rows}) state <= MV_GO;
        else if (aj < {16'd0, mv_blen} >> 4) aj <= aj + 32'd1;
        else state <= MV_ACT_WR;
      end
      MV_ACT_WR: if (!aq_busy) begin act_wr <= 1'b1; state <= MV_ACT_GAP; end
      MV_ACT_GAP: begin
        aj <= 32'd0;
        if ({16'd0, act_b} + 32'd1 == mv_blocks) begin act_b <= 16'd0; act_t <= act_t + 8'd1; end else act_b <= act_b + 16'd1;
        state <= MV_ACT;
      end
      MV_GO: begin mv_start <= 1'b1; mv_p <= 32'd0; mv_b <= 32'd0; state <= MV_NEXT; end
      MV_NEXT: begin
        if (mv_p == mv_pairs) state <= MV_DRAIN;
        else begin
          g_two <= 1'b0; g_i <= 4'd0;
          g_byte <= g_row_byte + {32'd0, mv_p} * 2 * mv_row_bytes;
          state <= MV_G;
        end
      end
      MV_G: begin
        g_n <= g_lines[3:0];
        if (g_i != 4'd0 && g_i == g_n) begin
          if (!g_two) begin
            w_blk1 <= g_shifted[@WIDEST@:0];
            g_two <= 1'b1; g_i <= 4'd0;
            // The pair's second row, or its first again past the matrix's last row.
            g_byte <= g_row_byte + ((2 * mv_p + 1 < mv_out) ? ({32'd0, mv_p} * 2 + 1) : ({32'd0, mv_p} * 2)) * mv_row_bytes;
          end else begin
            w_blk2 <= g_shifted[@WIDEST@:0];
            state <= MV_PRESENT;
          end
        end else begin
          rq_v <= 1'b1; rq_line <= g_byte[37:6] + g_i; state <= MV_GW;
        end
      end
      MV_GW: if (mem_rd_valid) begin
        gbuf[512 * g_i +: 512] <= mem_rd_data;
        g_i <= g_i + 4'd1;
        g_n <= g_lines[3:0];
        state <= MV_G;
      end
      MV_PRESENT: if (mv_ready) begin
        if (mv_b + 32'd1 == mv_blocks) begin mv_b <= 32'd0; mv_p <= mv_p + 32'd1; end else mv_b <= mv_b + 32'd1;
        state <= MV_NEXT;
      end
      MV_DRAIN: if (!mv_col_busy && !mv_start) state <= next_job;
"#;

const REFERENCE_MATVEC_READS: &str = r#"      MV_ACT: st_ra = mv_from_slot[31:0] + {24'd0, act_t} * mv_from_slot[63:32] + {16'd0, act_b} * ({16'd0, mv_blen} >> 4) + aj;
"#;

const REFERENCE_MATVEC_WRITES: &str = r#"    end else if (o_v) begin
      st_we = 1'b1; st_wa = o_beat;
      st_wd[32 * o_e[3:0] +: 32] = o_1; st_wm[4 * o_e[3:0] +: 4] = 4'hf;
      if (o_e + 32'd1 < mv_out) begin st_wd[32 * (o_e[3:0] + 4'd1) +: 32] = o_2; st_wm[4 * (o_e[3:0] + 4'd1) +: 4] = 4'hf; end
"#;

const REFERENCE_STORE_WRITE_NOTE: &str = r#"  // The store's write port: the host's beat; a column's two outputs under their lanes, the second past the matrix's
  // last row kept out; a store's lanes; the units' writes.
"#;

const REFERENCE_MATVEC_PULSES: &str = r#" mv_start <= 1'b0; act_wr <= 1'b0;"#;

/// The recurrence unit for heads of `hd` elements and logs of `t_max` tokens: its declarations, row engines and
/// wiring, and its states, each a Verilog fragment the engine's template takes. The vector unit gives each head's
/// `k·q`, and folds a lazily flushed head's query back through its log, an entry's instructions at a time, newest first.
/// A unit's Verilog for the engine's template: its declarations, its states, the store beat each of its states reads
/// (case items of the store's read address) and its writes into the store (branches of the store's write port).
#[derive(Default)]
struct UnitVerilog {
    decls: String,
    cases: String,
    reads: String,
    writes: String,
}

fn delta_verilog(hd: usize, t_max: usize) -> UnitVerilog {
    let w = |op: Op| format!("64'h{:016x}", super::simd::encode(&op));
    let fold = [
        Op::Dot { a: 2, b: 0, d: 3 },
        Op::Scalar { op: Bin::Mul, a: 2, b: 3, d: 3 },
        Op::VecScalar { op: Bin::Mul, a: 3, s: 3, d: 4 },
        Op::Vec { op: Bin::Add, a: 1, b: 4, d: 1 },
        Op::VecScalar { op: Bin::Mul, a: 2, s: 3, d: 5 },
        Op::Vec { op: Bin::Sub, a: 0, b: 5, d: 6 },
        Op::VecScalar { op: Bin::Mul, a: 6, s: 1, d: 0 },
    ];
    let mut fold_case = String::new();
    for (i, op) in fold.iter().enumerate() {
        writeln!(fold_case, "        3'd{i}: dl_word = {};", w(*op)).unwrap();
    }
    let in_values = (0..t_max).rev().map(|e| format!("dl_vals[{e}][32*dr +: 32]")).collect::<Vec<_>>().join(", ");
    let decls = format!(
        r#"  // The recurrence unit: the job's slots, its block's place in a region, the head at hand and its log.
  localparam HD = {hd}, TMAX = {t_max};
  reg [4:0] dl_qs, dl_ks, dl_vs, dl_lds, dl_dcs, dl_gts, dl_tos;
  reg [19:0] dl_vat;
  wire [511:0] dlb = bw;
  wire [31:0] dl_mem = dlb[127:96], dl_log = dlb[159:128], dl_entry = dlb[191:160];
  wire [31:0] dl_nk = dlb[223:192], dl_nv = dlb[255:224], dl_hd = dlb[287:256];
  wire [31:0] dl_fval = dlb[319:288], dl_fdec = dlb[351:320], dl_fgate = dlb[383:352], dl_T = dlb[415:384], dl_scale = dlb[447:416];
  wire [31:0] dl_beats = dl_hd >> 4;
  reg [31:0] dg, dr, dj, de, dl_count, dob;
  reg [1:0] dfld;
  reg [2:0] dl_ii;
  reg dl_flush;
  reg [32*HD-1:0] dl_kv, dl_qv, dl_y, dl_c;
  `LOGOS_RAMSTYLE_ASYNC(TMAX) reg [32*HD-1:0] dl_vals [0:TMAX-1];
  reg [31:0] dl_kq, dl_decay, dl_beta, dl_dec_e, dl_beta_e;
  reg [3:0] dl_rd_reg, dl_rd_s;
  reg [15:0] dl_rd_row;
  // The vector unit's read port is the fold's while its folded query and carried values are read out: a function of
  // the state, so no power-up value of a register of its own can hand it to the fold outside those states.
  wire dl_rd = state == DL_FY || state == DL_FC;
  // A slot's beat holding element `e` of the row at hand; the element is the beat's lane `e mod 16`.
  function [31:0] element_beat(input [63:0] slot, input [31:0] e);
    element_beat = slot[31:0] + {{16'd0, k}} * slot[63:32] + (e >> 4);
  endfunction
  wire [31:0] dl_v_e = {{12'd0, dl_vat}} + h * dl_hd + dr;
  wire [31:0] dl_v_elem = st_rd[32 * dl_v_e[3:0] +: 32];
  wire [31:0] dl_h_elem = st_rd[32 * h[3:0] +: 32];
  // A head's key or query beat: the group's head of the row at hand.
  wire [31:0] dl_ks_beat = slott[dl_ks][31:0] + {{16'd0, k}} * slott[dl_ks][63:32] + ((dg * dl_hd) >> 4) + dj;
  wire [31:0] dl_qs_beat = slott[dl_qs][31:0] + {{16'd0, k}} * slott[dl_qs][63:32] + ((dg * dl_hd) >> 4) + dj;
  // The state row at hand, and the line of its beat `dj`.
  wire [63:0] dl_row_byte = {{32'd0, region}} * 64 + dl_mem + ({{32'd0, h}} * dl_hd + dr) * dl_hd * st_bytes;
  wire [63:0] dl_beat_byte = dl_row_byte + dj * 16 * st_bytes;
  wire [63:0] dl_out_byte = dl_row_byte + dob * 16 * st_bytes;
  // The log: its count's line, the entry at hand, a field's beat.
  wire [63:0] dl_log_byte = {{32'd0, region}} * 64 + dl_log;
  wire [63:0] dl_entry_byte = dl_log_byte + 64 + de * dl_entry;
  reg [31:0] dl_src_slot_e, dl_dst_e, dl_len;
  reg [4:0] dl_src_slot;
  always @* begin
    case (dfld)
      2'd0: begin dl_src_slot = dl_ks; dl_src_slot_e = 32'd0; dl_dst_e = 32'd0; dl_len = dl_nk * dl_hd; end
      2'd1: begin dl_src_slot = dl_vs; dl_src_slot_e = {{12'd0, dl_vat}}; dl_dst_e = dl_fval; dl_len = dl_nv * dl_hd; end
      2'd2: begin dl_src_slot = dl_lds; dl_src_slot_e = 32'd0; dl_dst_e = dl_fdec; dl_len = dl_nv; end
      default: begin dl_src_slot = dl_gts; dl_src_slot_e = 32'd0; dl_dst_e = dl_fgate; dl_len = dl_nv; end
    endcase
  end
  wire [63:0] dl_app_byte = dl_log_byte + 64 + dl_count * dl_entry + (dl_dst_e + 16 * dj) * st_bytes;
  wire [31:0] dl_app_lanes = (dl_len - 16 * dj >= 32'd16) ? 32'd16 : dl_len - 16 * dj;
  wire [15:0] dl_app_mask = (dl_app_lanes >= 32'd16) ? 16'hffff : ((16'd1 << dl_app_lanes[4:0]) - 16'd1);
  wire [31:0] dl_app_src = slott[dl_src_slot][31:0] + {{16'd0, k}} * slott[dl_src_slot][63:32] + (dl_src_slot_e >> 4) + dj;
  wire [511:0] dl_app_beat = st_rd;
  // A state beat as the engine reads it: f32, or the line's half of binary16 widened.
  wire [511:0] dl_widened;
  wire [255:0] dl_halves, dl_app_halves;
  wire [511:0] dl_out_beat_data;
  genvar dgv;
  generate
    for (dgv = 0; dgv < 16; dgv = dgv + 1) begin : dl_lanes
      logos_f16_to_f32 wv(.h(dl_beat_byte[5] ? mem_rd_data[256 + 16*dgv +: 16] : mem_rd_data[16*dgv +: 16]), .x(dl_widened[32*dgv +: 32]));
      logos_f32_to_f16 nv(.x(dl_out_beat_data[32*dgv +: 32]), .h(dl_halves[16*dgv +: 16]));
      logos_f32_to_f16 na(.x(dl_app_beat[32*dgv +: 32]), .h(dl_app_halves[16*dgv +: 16]));
    end
  endgenerate
  // An entry's key or value beat at hand during the fold, and its line's half widened.
  wire [63:0] dl_fk_byte = dl_entry_byte + (dg * dl_hd + 16 * dj) * st_bytes;
  wire [63:0] dl_fv_byte = dl_entry_byte + (dl_fval + h * dl_hd + 16 * dj) * st_bytes;
  wire [63:0] dl_fbyte = (state == DL_FV || state == DL_FVW) ? dl_fv_byte : dl_fk_byte;
  wire [511:0] dl_fwide;
  generate
    for (dgv = 0; dgv < 16; dgv = dgv + 1) begin : dl_flanes
      logos_f16_to_f32 wf(.h(dl_fbyte[5] ? mem_rd_data[256 + 16*dgv +: 16] : mem_rd_data[16*dgv +: 16]), .x(dl_fwide[32*dgv +: 32]));
    end
  endgenerate
  // One element of a log entry's field, from its line: f32, or binary16 widened.
  wire [63:0] dl_elem_byte = dl_entry_byte + ((dfld == 2'd2 ? dl_fdec : dl_fgate) + h) * st_bytes;
  wire [31:0] dl_elem_f16;
  logos_f16_to_f32 we(.h(mem_rd_data[8 * dl_elem_byte[5:0] +: 16]), .x(dl_elem_f16));
  wire [31:0] dl_elem = state_f16 ? dl_elem_f16 : mem_rd_data[8 * dl_elem_byte[5:0] +: 32];
  // The row engines: every token's update, and a lazily flushed memory's read-out and replay.
  reg dre_v, dre_first, dlr_v, dlr_first, dlr_log_wr;
  reg [511:0] dre_data;
  wire dre_ov, dre_ol, dlr_ov, dlr_ol, dlr_busy;
  wire [511:0] dre_od, dlr_od;
  wire [31:0] dre_or, dlr_or;
  logos_gdn_row_engine #(.N(HD)) dre(.clk(clk), .rst(rst), .k(dl_kv), .q(dl_qv), .decay(dl_decay), .beta(dl_beta), .kq(dl_kq),
    .scale(dl_scale), .f16(state_f16), .in_valid(dre_v), .in_first(dre_first), .in_data(dre_data), .in_v(dl_v_elem),
    .out_valid(dre_ov), .out_last(dre_ol), .out_data(dre_od), .out_read(dre_or));
  logos_gdn_lazy_row #(.N(HD), .T_MAX(TMAX)) dlr(.clk(clk), .rst(rst), .y(dl_y), .scale(dl_scale), .f16(state_f16),
    .entries(dl_count[7:0]), .flush(dl_flush), .log_wr(dlr_log_wr), .log_at(de[7:0]), .log_k(dl_kv), .log_decay(dl_dec_e),
    .log_beta(dl_beta_e), .in_valid(dlr_v), .in_first(dlr_first), .in_data(dre_data), .in_carried(dl_c[32*dr +: 32]),
    .in_values({{{in_values}}}), .out_valid(dlr_ov), .out_last(dlr_ol), .out_data(dlr_od), .out_read(dlr_or), .busy(dlr_busy));
  wire dl_lazy = dl_T != 32'd0;
  wire dl_out_v = dl_lazy ? dlr_ov : dre_ov, dl_out_l = dl_lazy ? dlr_ol : dre_ol;
  assign dl_out_beat_data = dl_lazy ? dlr_od : dre_od;
  wire [31:0] dl_out_read = dl_lazy ? dlr_or : dre_or;
  // A fold's instruction.
  reg [63:0] dl_word;
  always @* begin
    case (dl_ii)
{fold_case}        default: dl_word = 64'd0;
    endcase
  end
  // The read-out's element of the job's output.
  wire [63:0] dl_to_slot = slott[dl_tos];
  wire [31:0] dl_to_e = h * dl_hd + dr;
  wire [31:0] dl_to_beat = dl_to_slot[31:0] + {{16'd0, k}} * dl_to_slot[63:32] + (dl_to_e >> 4);
"#
    );
    let cases = format!(
        r#"      DL_HDR: begin
        dl_qs <= jw[4:0]; dl_ks <= jw[9:5]; dl_vs <= jw[14:10]; dl_lds <= jw[19:15];
        dl_dcs <= jw[24:20]; dl_gts <= jw[29:25]; dl_tos <= jw[34:30]; dl_vat <= jw[54:35];
        pc <= pc + 32'd1; k <= 16'd0; state <= DL_ROW;
      end
      DL_ROW: begin
        if (k == rows) state <= next_job;
        else begin h <= 32'd0; dg <= 32'd0; dj <= 32'd0; state <= dl_lazy ? DL_CNT : DL_KQ_K; end
      end
      // Every token's update: the head's k·q on the vector unit, then each value row through the row engine.
      DL_KQ_K: begin
        if (dj == dl_beats) begin dj <= 32'd0; state <= DL_KQ_Q; end
        else begin
          s_wr <= 1'b1; s_wr_reg <= 4'd0; s_wr_row <= dj[15:0];
          s_wr_data <= st_rd;
          dl_kv[512 * dj +: 512] <= st_rd;
          dj <= dj + 32'd1;
        end
      end
      DL_KQ_Q: begin
        if (dj == dl_beats) begin dj <= 32'd0; state <= DL_KQ_L0; end
        else begin
          s_wr <= 1'b1; s_wr_reg <= 4'd1; s_wr_row <= dj[15:0];
          s_wr_data <= st_rd;
          dl_qv[512 * dj +: 512] <= st_rd;
          dj <= dj + 32'd1;
        end
      end
      DL_KQ_L0: begin s_len_wr <= 1'b1; s_wr_reg <= 4'd0; s_len_val <= dl_hd[19:0]; state <= DL_KQ_L1; end
      DL_KQ_L1: begin s_len_wr <= 1'b1; s_wr_reg <= 4'd1; s_len_val <= dl_hd[19:0]; state <= DL_KQ_DOT; end
      DL_KQ_DOT: begin s_issue <= 1'b1; s_instr <= {dot}; state <= DL_KQ_GAP; end
      DL_KQ_GAP: state <= DL_KQ_WAIT;
      DL_KQ_WAIT: if (!s_busy) begin
        dl_kq <= s_rd_s_val; dl_decay <= dl_h_elem; state <= DL_KQ_BETA;
      end
      DL_KQ_BETA: begin dl_beta <= dl_h_elem; dr <= 32'd0; dj <= 32'd0; state <= DL_E_RD; end
      DL_E_RD: begin
        if (dj == dl_beats) begin dob <= 32'd0; state <= DL_E_OUT; end
        else begin rq_v <= 1'b1; rq_line <= dl_beat_byte[37:6]; state <= DL_E_RW; end
      end
      DL_E_RW: if (mem_rd_valid) begin
        dre_v <= 1'b1; dre_first <= (dj == 32'd0); dre_data <= state_f16 ? dl_widened : mem_rd_data;
        dj <= dj + 32'd1; state <= DL_E_RD;
      end
      DL_E_OUT, DL_L_OUT: begin
        if (dl_out_v && !(dl_lazy && !dl_flush)) begin
          // An updated beat back where it was read, as the state stores it.
          wq_v <= 1'b1; wq_line <= dl_out_byte[37:6];
          wq_data <= 512'd0; wq_mask <= 64'd0;
          if (state_f16) begin
            wq_data <= {{256'd0, dl_halves}} << (8 * dl_out_byte[5:0]);
            wq_mask <= {{32'd0, 32'hffff_ffff}} << dl_out_byte[5:0];
          end else begin wq_data <= dl_out_beat_data; wq_mask <= 64'hffff_ffff_ffff_ffff; end
          dob <= dob + 32'd1;
        end
        if (dl_out_l) begin
          dj <= 32'd0;
          if (dr + 32'd1 == dl_hd) begin
            dr <= 32'd0;
            if (h + 32'd1 == dl_nv) begin
              if (dl_lazy) state <= DL_CNT_WR;
              else begin k <= k + 16'd1; state <= DL_ROW; end
            end else begin
              h <= h + 32'd1; dg <= (dg + 32'd1 == dl_nk) ? 32'd0 : dg + 32'd1;
              state <= dl_lazy ? DL_FQ : DL_KQ_K;
            end
          end else begin dr <= dr + 32'd1; state <= dl_lazy ? DL_L_RD : DL_E_RD; end
        end
      end
      // A lazily flushed memory: the row's token logged after the count, the query folded back through the log
      // for each head, each value row read through the folded query and, once the log is full, replayed and stored.
      DL_CNT: begin rq_v <= 1'b1; rq_line <= dl_log_byte[37:6]; state <= DL_CNT_W; end
      DL_CNT_W: if (mem_rd_valid) begin dl_count <= mem_rd_data[31:0]; dfld <= 2'd0; dj <= 32'd0; state <= DL_APP; end
      DL_APP: begin
        if (16 * dj >= dl_len) begin
          dj <= 32'd0;
          if (dfld == 2'd3) begin
            dl_count <= dl_count + 32'd1; dl_flush <= (dl_count + 32'd1 >= dl_T); h <= 32'd0; dg <= 32'd0; state <= DL_FQ;
          end else dfld <= dfld + 2'd1;
        end else if (wq_room) begin
          wq_v <= 1'b1; wq_line <= dl_app_byte[37:6];
          wq_data <= 512'd0; wq_mask <= 64'd0;
          if (state_f16) begin
            wq_data <= {{256'd0, dl_app_halves}} << (8 * dl_app_byte[5:0]);
            for (i = 0; i < 16; i = i + 1) if (dl_app_mask[i]) wq_mask[dl_app_byte[5:0] + 2*i +: 2] <= 2'b11;
          end else begin
            wq_data <= dl_app_beat;
            for (i = 0; i < 16; i = i + 1) if (dl_app_mask[i]) wq_mask[4*i +: 4] <= 4'hf;
          end
          dj <= dj + 32'd1;
        end
      end
      DL_FQ: begin
        // The fold starts from the head's group's query and no values carried.
        if (dj == dl_beats) begin dj <= 32'd0; state <= DL_FZ; end
        else begin
          s_wr <= 1'b1; s_wr_reg <= 4'd0; s_wr_row <= dj[15:0];
          s_wr_data <= st_rd;
          dj <= dj + 32'd1;
        end
      end
      DL_FZ: begin
        if (dj == dl_beats) begin dj <= 32'd0; state <= DL_FL0; end
        else begin s_wr <= 1'b1; s_wr_reg <= 4'd1; s_wr_row <= dj[15:0]; s_wr_data <= 512'd0; dj <= dj + 32'd1; end
      end
      DL_FL0: begin s_len_wr <= 1'b1; s_wr_reg <= 4'd0; s_len_val <= dl_hd[19:0]; state <= DL_FL1; end
      DL_FL1: begin s_len_wr <= 1'b1; s_wr_reg <= 4'd1; s_len_val <= dl_hd[19:0]; de <= dl_count - 32'd1; dj <= 32'd0; state <= DL_FK; end
      DL_FK: begin
        if (dj == dl_beats) begin dj <= 32'd0; state <= DL_FV; end
        else begin rq_v <= 1'b1; rq_line <= dl_fk_byte[37:6]; state <= DL_FKW; end
      end
      DL_FKW: if (mem_rd_valid) begin
        s_wr <= 1'b1; s_wr_reg <= 4'd2; s_wr_row <= dj[15:0];
        s_wr_data <= state_f16 ? dl_fwide : mem_rd_data; dl_kv[512 * dj +: 512] <= state_f16 ? dl_fwide : mem_rd_data;
        dj <= dj + 32'd1; state <= DL_FK;
      end
      DL_FV: begin
        if (dj == dl_beats) begin dj <= 32'd0; dfld <= 2'd2; state <= DL_FDEC; end
        else begin rq_v <= 1'b1; rq_line <= dl_fv_byte[37:6]; state <= DL_FVW; end
      end
      DL_FVW: if (mem_rd_valid) begin
        s_wr <= 1'b1; s_wr_reg <= 4'd3; s_wr_row <= dj[15:0];
        s_wr_data <= state_f16 ? dl_fwide : mem_rd_data; dl_vals[de][512 * dj +: 512] <= state_f16 ? dl_fwide : mem_rd_data;
        dj <= dj + 32'd1; state <= DL_FV;
      end
      DL_FDEC: begin rq_v <= 1'b1; rq_line <= dl_elem_byte[37:6]; state <= DL_FDECW; end
      DL_FDECW: if (mem_rd_valid) begin
        // The decay exponent as a constant, its exp the entry's decay; then the gate the same way.
        s_issue <= 1'b1; s_instr <= {{dl_elem, 32'h0001_0008}}; state <= DL_FDEC_GAP;
      end
      DL_FDEC_GAP: state <= DL_FDEC_WAIT;
      DL_FDEC_WAIT: if (!s_busy) begin s_issue <= 1'b1; s_instr <= {exp}; state <= DL_FEXP_GAP; end
      DL_FEXP_GAP: state <= DL_FEXP_WAIT;
      DL_FEXP_WAIT: if (!s_busy) begin dl_dec_e <= s_rd_s_val; dfld <= 2'd3; state <= DL_FBETA; end
      DL_FBETA: begin rq_v <= 1'b1; rq_line <= dl_elem_byte[37:6]; state <= DL_FBETAW; end
      DL_FBETAW: if (mem_rd_valid) begin
        dl_beta_e <= dl_elem; s_issue <= 1'b1; s_instr <= {{dl_elem, 32'h0002_0008}}; state <= DL_FB_GAP;
      end
      DL_FB_GAP: state <= DL_FB_WAIT;
      DL_FB_WAIT: if (!s_busy) begin s_len_wr <= 1'b1; s_wr_reg <= 4'd2; s_len_val <= dl_hd[19:0]; state <= DL_FL3; end
      DL_FL3: begin s_len_wr <= 1'b1; s_wr_reg <= 4'd3; s_len_val <= dl_hd[19:0]; dl_ii <= 3'd0; state <= DL_FRUN; end
      DL_FRUN: begin
        if (dl_ii == 3'd7) begin dlr_log_wr <= 1'b1; state <= DL_FLOG; end
        else begin s_issue <= 1'b1; s_instr <= dl_word; dl_ii <= dl_ii + 3'd1; state <= DL_FR_GAP; end
      end
      DL_FR_GAP: state <= DL_FR_WAIT;
      DL_FR_WAIT: if (!s_busy) state <= DL_FRUN;
      DL_FLOG: begin
        if (de == 32'd0) begin dj <= 32'd0; dl_rd_reg <= 4'd0; dl_rd_row <= 16'd0; state <= DL_FY; end
        else begin de <= de - 32'd1; dj <= 32'd0; state <= DL_FK; end
      end
      DL_FY: begin
        // The folded query and the values carried, read out of the vector unit a beat a cycle.
        if (dj == dl_beats) begin dj <= 32'd0; dl_rd_reg <= 4'd1; dl_rd_row <= 16'd0; state <= DL_FC; end
        else begin dl_y[512 * dj +: 512] <= s_rd_data; dj <= dj + 32'd1; dl_rd_row <= dl_rd_row + 16'd1; end
      end
      DL_FC: begin
        if (dj == dl_beats) begin dj <= 32'd0; dr <= 32'd0; state <= DL_L_RD; end
        else begin dl_c[512 * dj +: 512] <= s_rd_data; dj <= dj + 32'd1; dl_rd_row <= dl_rd_row + 16'd1; end
      end
      DL_L_RD: begin
        if (dj == dl_beats) begin dob <= 32'd0; state <= DL_L_OUT; end
        else begin rq_v <= 1'b1; rq_line <= dl_beat_byte[37:6]; state <= DL_L_RW; end
      end
      DL_L_RW: if (mem_rd_valid) begin
        dlr_v <= 1'b1; dlr_first <= (dj == 32'd0); dre_data <= state_f16 ? dl_widened : mem_rd_data;
        dj <= dj + 32'd1; state <= DL_L_RD;
      end
      DL_CNT_WR: if (wq_room) begin
        wq_v <= 1'b1; wq_line <= dl_log_byte[37:6];
        wq_data <= {{480'd0, dl_flush ? 32'd0 : dl_count}}; wq_mask <= 64'h0000_0000_0000_000f;
        k <= k + 16'd1; state <= DL_ROW;
      end
"#,
        dot = w(Op::Dot { a: 0, b: 1, d: 0 }),
        exp = w(Op::ScalarMap { f: Func::Exp, a: 1, d: 1 }),
    );
    let reads = r#"      DL_KQ_K: st_ra = dl_ks_beat;
      DL_KQ_Q, DL_FQ: st_ra = dl_qs_beat;
      DL_KQ_WAIT: st_ra = element_beat(slott[dl_dcs], h);
      DL_KQ_BETA: st_ra = element_beat(slott[dl_gts], h);
      DL_E_RD, DL_E_RW, DL_E_OUT: st_ra = element_beat(slott[dl_vs], dl_v_e);
      DL_APP: st_ra = dl_app_src;
"#
    .to_string();
    let writes = r#"    else if (!rst && (state == DL_E_OUT || state == DL_L_OUT) && dl_out_l) begin
      st_we = 1'b1; st_wa = dl_to_beat; st_wd[32 * dl_to_e[3:0] +: 32] = dl_out_read; st_wm[4 * dl_to_e[3:0] +: 4] = 4'hf;
    end
"#
    .to_string();
    UnitVerilog { decls, cases, reads, writes }
}

/// The attention unit for query heads of `hd` elements and value heads of `vd` over up to `p_max` positions, its math
/// on the array over an int8 cache when `array`, on the vector units otherwise: its declarations and its states, each
/// a Verilog fragment the engine's template takes. A query head's cached rows stream from the row's sequence's region
/// as the unit asks for them: binary16 elements widened, or an int8 row's codes and its scale — the codes times the
/// scale for the vector units, as they are for the array.
fn attend_unit_verilog(hd: usize, vd: usize, p_max: usize, array: bool) -> UnitVerilog {
    let unit = if array {
        format!(
            r#"  logos_attend_int8 #(.HD(AH), .VD(AV), .P_MAX(AP)) atu(.clk(clk), .rst(rst), .start(at_start), .positions(at_n[15:0]),
    .q_codes(aq_codes[16*AH-1:0]), .q_m(aq_m), .q_e(aq_e), .scale(at_scale), .busy(at_busy),
    .k_valid(at_kv), .k_codes(at_krow[8*AH-1:0]), .k_scale(at_kscale), .k_ready(at_kr),
    .s_valid(at_sv), .v_scale(at_vscale), .s_ready(at_sr),
    .v_valid(at_vv), .v_codes(at_vrow[8*AV-1:0]), .v_ready(at_vr),
    .o_valid(at_ov), .o_beat(at_ob), .o_last(at_ol));"#
        )
    } else {
        r#"  assign at_sr = 1'b0;
  logos_attend_vector #(.HD(AH), .VD(AV), .P_MAX(AP)) atu(.clk(clk), .rst(rst), .start(at_start), .positions(at_n[15:0]),
    .q(at_q), .scale(at_scale), .busy(at_busy),
    .k_valid(at_kv), .k_beat(at_beat), .k_ready(at_kr),
    .v_valid(at_vv), .v_beat(at_beat), .v_ready(at_vr),
    .o_valid(at_ov), .o_beat(at_ob), .o_last(at_ol));"#
            .to_string()
    };
    let decls = format!(
        r#"  // The attention unit: the job's slots, its block's cache, the row and head at hand, and the cache rows it streams.
  localparam AH = {hd}, AV = {vd}, AP = {p_max};
  // A head's beats, sixteen elements a beat, and the lines its int8 codes take.
  localparam [31:0] AH_BEATS = AH / 16, AV_BEATS = AV / 16, AH_LINES = (AH + 63) / 64, AV_LINES = (AV + 63) / 64;
  reg [4:0] at_qs, at_tos;
  reg [15:0] at_heads;
  reg [31:0] at_scale, at_g, at_n, at_kj, at_kb, at_sj, at_vj, at_vb, at_ob_i, at_lj;
  reg [1:0] at_what;
  reg [32*AH-1:0] at_q;
  reg at_start, at_kv, at_sv, at_vv;
  reg [511:0] at_line, at_beat;
  reg [8*AH-1:0] at_krow;
  reg [8*AV-1:0] at_vrow;
  reg [15:0] at_kscale, at_vscale, at_rowscale;
  wire at_busy, at_kr, at_sr, at_vr, at_ov, at_ol;
  wire [511:0] at_ob;
  wire [511:0] atb = bw;
  wire [31:0] at_keys = atb[31:0], at_vals = atb[63:32], at_kscales = atb[95:64], at_vscales = atb[127:96];
  wire [31:0] at_kvh = atb[159:128];
  // A cached head row's first byte, keys or values, and its scale's: position j's row of the cached head g.
  wire [31:0] at_kw = cache_int8 ? AH : 2 * AH, at_vw = cache_int8 ? AV : 2 * AV;
  wire [63:0] at_kbyte = {{32'd0, region}} * 64 + at_keys + ({{32'd0, at_kj}} * at_kvh + at_g) * at_kw;
  wire [63:0] at_vbyte = {{32'd0, region}} * 64 + at_vals + ({{32'd0, at_vj}} * at_kvh + at_g) * at_vw;
  wire [63:0] at_ksbyte = {{32'd0, region}} * 64 + at_kscales + ({{32'd0, at_kj}} * at_kvh + at_g) * 2;
  wire [63:0] at_vsbyte = {{32'd0, region}} * 64 + at_vscales + ({{32'd0, at_what == 2'd1 ? at_sj : at_vj}} * at_kvh + at_g) * 2;
  // The vector units' beat of a fetched line: its binary16 half widened, or sixteen int8 codes times their row's scale.
  wire [63:0] at_beat_byte = at_what == 2'd0 ? at_kbyte + at_kb * (cache_int8 ? 16 : 32) : at_vbyte + at_vb * (cache_int8 ? 16 : 32);
  wire [511:0] at_wide, at_deq;
  wire [31:0] at_scale_f;
  logos_f16_to_f32 asf(.h(at_rowscale), .x(at_scale_f));
  genvar agv;
  generate
    for (agv = 0; agv < 16; agv = agv + 1) begin : at_lanes
      logos_f16_to_f32 aw(.h(at_beat_byte[5] ? at_line[256 + 16*agv +: 16] : at_line[16*agv +: 16]), .x(at_wide[32*agv +: 32]));
      wire [7:0] code = at_line[8 * ({{2'd0, at_beat_byte[5:4]}} * 16 + agv) +: 8];
      wire [31:0] cf;
      logos_i2f ai(.n({{{{24{{code[7]}}}}, code}}), .y(cf));
      logos_fmul am(.a(cf), .b(at_scale_f), .y(at_deq[32*agv +: 32]));
    end
  endgenerate
  // The head's output beat in the job's output slot.
  wire [63:0] at_to_slot = slott[at_tos];
  wire [31:0] at_to_beat = at_to_slot[31:0] + {{16'd0, k}} * at_to_slot[63:32] + ((h * AV) >> 4) + at_ob_i;
{unit}
"#
    );
    let common = r#"      AT_HDR: begin
        at_qs <= jw[4:0]; at_tos <= jw[9:5]; at_heads <= jw[25:10]; at_scale <= jw[57:26];
        pc <= pc + 32'd1; k <= 16'd0; state <= AT_ROW;
      end
      AT_ROW: begin
        if (k == rows) state <= next_job;
        else begin h <= 32'd0; state <= AT_HEAD; end
      end
      AT_HEAD: begin
        // The head's cached head, and its row's positions to its own.
        at_g <= h / ({16'd0, at_heads} / at_kvh); at_n <= position + 32'd1; at_kj <= 32'd0; at_kb <= 32'd0; at_sj <= 32'd0;
        at_vj <= 32'd0; at_vb <= 32'd0; at_ob_i <= 32'd0; at_lj <= 32'd0; state <= AT_Q;
      end
      AT_Q: begin
        // The head's query, into the unit and the activation quantizer.
        if (at_lj == AH_BEATS) begin at_lj <= 32'd0; state <= AT_Q_GAP; end
        else begin
          at_q[512 * at_lj +: 512] <= st_rd;
          at_lj <= at_lj + 32'd1;
        end
      end
      AT_Q_GAP: if (!aq_busy) begin at_start <= 1'b1; state <= AT_RUN; end
      AT_TAKEN: begin
        // The unit took the item on the edge it was offered: the stream moves to its next position, or beat.
        if (at_kv) begin
          at_kv <= 1'b0;
          if (AT_BEATS && at_kb + 32'd1 != AH_BEATS) at_kb <= at_kb + 32'd1; else begin at_kb <= 32'd0; at_kj <= at_kj + 32'd1; end
        end
        if (at_sv) begin at_sv <= 1'b0; at_sj <= at_sj + 32'd1; end
        if (at_vv) begin
          at_vv <= 1'b0;
          if (AT_BEATS && at_vb + 32'd1 != AV_BEATS) at_vb <= at_vb + 32'd1; else begin at_vb <= 32'd0; at_vj <= at_vj + 32'd1; end
        end
        state <= AT_RUN;
      end
"#;
    let run = r#"      AT_RUN: begin
        // Serve the unit whatever it is ready for, each stream its next position; its output beats into the store.
        if (at_ov) at_ob_i <= at_ob_i + 32'd1;
        if (at_ol) begin
          if (h + 32'd1 == {16'd0, at_heads}) begin k <= k + 16'd1; state <= AT_ROW; end
          else begin h <= h + 32'd1; state <= AT_HEAD; end
        end
        else if (at_kr) begin at_what <= 2'd0; at_lj <= 32'd0; state <= AT_FETCH; end
        else if (at_sr) begin at_what <= 2'd1; state <= AT_SCALE; end
        else if (at_vr) begin at_what <= 2'd2; at_lj <= 32'd0; state <= AT_FETCH; end
      end
      AT_SCALE: begin rq_v <= 1'b1; rq_line <= at_vsbyte[37:6]; state <= AT_SCALE_W; end
      AT_SCALE_W: if (mem_rd_valid) begin at_vscale <= mem_rd_data[8 * at_vsbyte[5:0] +: 16]; at_sv <= 1'b1; state <= AT_TAKEN; end
"#;
    let fetch = if array {
        r#"      AT_FETCH: begin
        // A row's codes, a line at a time; then a key row's scale.
        if (at_lj == (at_what == 2'd0 ? AH_LINES : AV_LINES)) begin
          if (at_what == 2'd0) begin rq_v <= 1'b1; rq_line <= at_ksbyte[37:6]; state <= AT_KS; end
          else begin at_vv <= 1'b1; state <= AT_TAKEN; end
        end else begin
          rq_v <= 1'b1; rq_line <= (at_what == 2'd0 ? at_kbyte[37:6] : at_vbyte[37:6]) + at_lj; state <= AT_FETCH_W;
        end
      end
      AT_FETCH_W: if (mem_rd_valid) begin
        if (at_what == 2'd0) at_krow[512 * at_lj +: 512] <= mem_rd_data; else at_vrow[512 * at_lj +: 512] <= mem_rd_data;
        at_lj <= at_lj + 32'd1; state <= AT_FETCH;
      end
      AT_KS: if (mem_rd_valid) begin at_kscale <= mem_rd_data[8 * at_ksbyte[5:0] +: 16]; at_kv <= 1'b1; state <= AT_TAKEN; end
"#
    } else {
        r#"      AT_FETCH: begin rq_v <= 1'b1; rq_line <= at_beat_byte[37:6]; state <= AT_FETCH_W; end
      AT_FETCH_W: if (mem_rd_valid) begin
        at_line <= mem_rd_data;
        if (cache_int8) begin
          rq_v <= 1'b1; rq_line <= (at_what == 2'd0 ? at_ksbyte[37:6] : at_vsbyte[37:6]); state <= AT_KS;
        end else state <= AT_GIVE;
      end
      AT_KS: if (mem_rd_valid) begin
        at_rowscale <= mem_rd_data[8 * (at_what == 2'd0 ? at_ksbyte[5:0] : at_vsbyte[5:0]) +: 16]; state <= AT_GIVE;
      end
      AT_GIVE: begin
        at_beat <= cache_int8 ? at_deq : at_wide;
        if (at_what == 2'd0) at_kv <= 1'b1; else at_vv <= 1'b1;
        state <= AT_TAKEN;
      end
"#
    };
    let cases = format!("{common}{run}{fetch}").replace("AT_BEATS", if array { "1'b0" } else { "1'b1" });
    let reads = "      AT_Q: st_ra = slott[at_qs][31:0] + {16'd0, k} * slott[at_qs][63:32] + ((h * AH) >> 4) + at_lj;\n".to_string();
    let writes = r#"    else if (!rst && state == AT_RUN && at_ov) begin
      st_we = 1'b1; st_wa = at_to_beat; st_wd = at_ob; st_wm = {64{1'b1}};
    end
"#
    .to_string();
    UnitVerilog { decls, cases, reads, writes }
}
