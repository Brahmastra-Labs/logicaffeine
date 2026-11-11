//! The step-local memory of a lane's forms. Every array a step computes and reads within the step — a tensor's `f32`
//! (or `i32`) buffer, or its quantized form — lives in one arena the program allocates once, placed by its lifetime:
//! from the launch that first writes it whole to the last launch that touches it, through any view of it. A launch's
//! scratch lives there while the launch runs. The forms share the arena, one form running at a time.

use std::collections::BTreeMap;

use super::kernels;
use super::plan::{quant_in_place, Fill, Launch, Schedule, Touch, QBLOCK};
use crate::codegen_cuda::qwen_emit::Q8MatKind;

/// Every array's alignment in the arena: `cudaMalloc`'s, past every vector load the kernels make.
pub const ALIGN: usize = 256;

/// An array placed in the arena: `bytes` from `offset`, live from launch `first` to launch `last` of its form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placed {
    pub name: String,
    pub touch: Touch,
    pub offset: usize,
    pub bytes: usize,
    pub first: usize,
    pub last: usize,
}

/// A launch's scratch placed in the arena: `bytes` from `offset`, live while launch `launch` of its form runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlacedScratch {
    pub launch: usize,
    pub tag: &'static str,
    pub offset: usize,
    pub bytes: usize,
}

/// A form's arena: its placed arrays, its launches' placed scratch, and the bytes they span.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Arena {
    pub placed: Vec<Placed>,
    pub scratch: Vec<PlacedScratch>,
    pub bytes: usize,
}

impl Arena {
    /// Where tensor `name`'s array `touch` lives, when it is placed.
    pub fn at(&self, name: &str, touch: Touch) -> Option<&Placed> {
        self.placed.iter().find(|p| p.name == name && p.touch == touch)
    }

    /// Where launch `launch`'s scratch `tag` lives.
    pub fn scratch_at(&self, launch: usize, tag: &str) -> Option<&PlacedScratch> {
        self.scratch.iter().find(|p| p.launch == launch && p.tag == tag)
    }
}

/// The tensor whose array a touch of tensor `name` lands in: an alias's — a view's — is the tensor it is a part of,
/// through every alias, but the quantized form of an alias whose rows start inside a block is its own.
pub fn home<'a>(s: &'a Schedule, name: &'a str, touch: Touch) -> &'a str {
    let mut at = name;
    while let Some(Fill::Alias { of, offset }) = s.buffers.get(at).map(|b| &b.fill) {
        if touch == Touch::Quant && !quant_in_place(*offset) {
            break;
        }
        at = of;
    }
    at
}

/// A launch's scratch: `tag` names it beside the launch's result, `ty` is its element type and `bytes` its size. A
/// counter — an arrival count the launch's last block resets — is zero before the launch and left zero by it, so it
/// keeps a zeroed buffer of its own; the rest live in the arena while the launch runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scratch {
    pub tag: &'static str,
    pub ty: &'static str,
    pub bytes: usize,
    pub counter: bool,
}

/// The scratch launch `l` takes under `numerics`: the delta rule's unnormed outputs; an expert mixture's router logits,
/// choices and weights, and over a block's rows its choices grouped by expert, each choice's gathered activation row,
/// hidden row (gate and up rows apart on the tensor cores, the hidden row quantized alone when they are paired) and
/// weighed result; a tensor-core projection's parts of the tiles its last wave cuts along `k`, and their counts; an
/// `f32` GEMM's parts of the tiles it cuts along `k`; a row normed by parts' sums of squares; the arg-max's block
/// maxima; a split attention's chunk partials (none in tiles, but the f16 tensor cores' stream-K parts); an attention
/// in two words' keys and values in two words and their scales — and the arrival counts.
pub fn scratch(l: &Launch, numerics: Q8MatKind) -> Vec<Scratch> {
    let m = kernels::mode(numerics);
    let two = numerics == Q8MatKind::TwoPassInt8;
    let (acc, acc_ty) = kernels::accumulator(m);
    let t = |tag, ty, bytes| Scratch { tag, ty, bytes, counter: false };
    let counter = |tag, bytes| Scratch { tag, ty: "int", bytes, counter: true };
    // A quantized activation of `n` values: its words, its low words under two words, its scales.
    let quantized = |v: &mut Vec<Scratch>, [q, lo, sc]: [&'static str; 3], n: usize| {
        v.push(t(q, "signed char", n));
        if two {
            v.push(t(lo, "signed char", n));
        }
        v.push(t(sc, "float", n / QBLOCK * 4));
    };
    match l {
        // In chunks: each chunk's decayed keys and queries, `T·diag(β)`, masked scores and decays beside the outputs.
        Launch::Delta { hv, d, rows, .. } if l.delta_in_chunks(numerics) => {
            let [kg, qg, tb, sc, dec] = kernels::delta_chunk_scratch(rows.div_ceil(kernels::DELTA_CHUNK), *hv, *d);
            vec![
                t("o", "float", rows * hv * d * 4),
                t("dk", "float", kg * 4),
                t("dq", "float", qg * 4),
                t("dt", "float", tb * 4),
                t("dp", "float", sc * 4),
                t("dg", "float", dec * 4),
            ]
        }
        // Stepped a row at a time: its unnormed outputs, and each head's count of blocks arrived — a sequence's own — for
        // the last to norm the head.
        Launch::Delta { hv, d, rows, seqs, .. } => vec![t("o", "float", rows * hv * d * seqs * 4), counter("dc", hv * seqs * 4)],
        // An `f32` GEMM cut along `k`: each part's sums, and each tile's count of parts arrived.
        Launch::GemvF32 { n, m, split, .. } if *split > 1 && l.multiplies_in_tiles(numerics) => {
            let tiles = n.div_ceil(64) * m.div_ceil(64);
            vec![t("fp", "float", tiles * split * 256 * 16 * 4), counter("fq", tiles * 4)]
        }
        Launch::Moe { d, dff, k, experts, rows, router_split, .. } if *rows > 1 => {
            let choices = rows * k;
            let mut v = vec![
                t("i", "int", choices * 4),
                t("g", "float", choices * 4),
                t("rl", "float", rows * experts * 4),
                counter("rc", rows * 4),
                t("ec", "int", experts * 4),
                t("eo", "int", (experts + 1) * 4),
                t("el", "int", choices * 4),
                t("ep", "int", choices * 4),
                t("pa", acc_ty, choices * d * acc),
            ];
            // The hidden rows `f32`, unless the paired GEMM quantizes them in its epilogue.
            if !l.pairs_gate_and_up(numerics) {
                v.push(t("h", "float", choices * dff * 4));
            }
            if m == 0 {
                v.push(t("gx", "float", choices * d * 4));
            } else {
                quantized(&mut v, ["gq", "gl", "gs"], choices * d);
                quantized(&mut v, ["hq", "hl", "hs"], choices * dff);
            }
            if l.on_tensor_cores(numerics) && !l.pairs_gate_and_up(numerics) {
                v.extend([t("hg", "float", choices * dff * 4), t("hu", "float", choices * dff * 4)]);
            }
            // An `f32` router's logits GEMM cut along `k`: each part's sums, and each tile's count of parts arrived.
            if *router_split > 1 {
                let tiles = experts.div_ceil(64) * rows.div_ceil(64);
                v.extend([t("rp", "float", tiles * router_split * 256 * 16 * 4), counter("rq", tiles * 4)]);
            }
            v
        }
        Launch::Moe { dff, k, experts, shared, .. } => {
            let hidden = k * dff + shared.as_ref().map_or(0, |e| e.dff);
            let mut v = vec![
                t("i", "int", k * 4),
                t("g", "float", k * 4),
                t("h", "float", hidden * 4),
                t("rl", "float", experts * 4),
                counter("rc", 4),
            ];
            if m > 0 {
                quantized(&mut v, ["hq", "hl", "hs"], hidden);
            }
            v
        }
        // A tensor-core projection's tiles cut along `k` (a GEMM in parts' over its parts together): each part's sums,
        // and each cut tile's count of parts arrived.
        Launch::Gemv { m, tail: Some((whole, split)), .. }
        | Launch::GemmParts { m, tail: Some((whole, split)), .. }
        | Launch::GemvGlu { m, tail: Some((whole, split)), .. }
            if l.on_tensor_cores(numerics) =>
        {
            let rows = l.gemm_rows(numerics);
            let tiles = l.gemm_tiles_wide() * m.div_ceil(rows);
            let parts = (tiles - whole) * split;
            vec![t("tp", "float", parts * rows * kernels::GEMM_TILE_N * 4), counter("tc", (tiles - whole) * 4)]
        }
        // A gated delta layer's inputs: its products' scratch, and each normed head's count of rows written.
        Launch::Fold { .. } if l.gdn_inputs().is_some() => {
            let Some((products, Launch::ConvParts { parts, block, .. }, _)) = l.gdn_inputs() else { unreachable!("a gated delta layer's inputs") };
            let mut v = scratch(products, numerics);
            v.push(counter("gc", parts.iter().filter(|c| c.l2).map(|c| c.chan / block).sum::<usize>().max(1) * 4));
            v
        }
        Launch::Fold { first, then } => {
            let mut v = scratch(first, numerics);
            v.extend(match (first.as_ref(), then.as_ref()) {
                // A gather that norms each row in the block that gathers it counts nothing.
                (Launch::Gather { .. }, _) => None,
                // Projections that turn their heads as they finish: each head's count of rows written.
                (_, Launch::RopeParts { sides, .. }) => Some(counter("gc", sides.iter().map(|s| s.heads).sum::<usize>() * 4)),
                // A mixture's sum with another array is made where the mixture writes its result.
                (_, Launch::Binary { .. }) => None,
                // A chunk's mixture norms each row in the block that combines it.
                _ => match l.finished_mixture() {
                    Some((Launch::Moe { rows, .. }, ..)) if *rows > 1 => None,
                    // A projection or the decode step's mixture that norms in its last block: the count of its blocks arrived.
                    _ => Some(counter("gc", 4)),
                },
            });
            v
        }
        // A batch's gated pair on the tensor cores: its gate's and its up's sums, gated after.
        Launch::GemvGlu { n, m, .. } if l.batch_on_tensor_cores(numerics) => vec![t("mg", "float", m * n * 4), t("mu", "float", m * n * 4)],
        // A row normed by parts: each row's parts' sums of squares, in the accumulator.
        Launch::NormParts { parts, rows, .. } => vec![t("ss", acc_ty, rows * parts.len() * acc)],
        Launch::ArgMax { n, rows, .. } => {
            let blocks = kernels::argmax_blocks(*n);
            vec![t("av", "float", rows * blocks * 4), t("ai", "int", rows * blocks * 4), counter("ac", rows * 4)]
        }
        // In two words: every key and value row the attention reads, its high and low words, and each row's two scales.
        Launch::Attention { heads, group, hd, vd, len, .. } if l.attends_in_two_words(numerics) => {
            let [k, v, sc] = kernels::attn_i8_scratch(*len, heads / group, *hd, *vd);
            vec![t("ak", "signed char", k), t("aw", "signed char", v), t("as", "float", sc)]
        }
        // On the f16 tensor cores under the stream-K schedule, with items to split: two partials a stream-K block, and
        // each item's count of parts arrived.
        Launch::Attention { heads, group, vd, rows, stream, .. } if *stream > 0 && l.attends_on_tensor_cores(numerics) => {
            let items = kernels::attn_mma_items(*rows, *heads, kernels::attn_tile_heads(*group));
            let (ndp, grid) = kernels::attn_mma_schedule(items, *stream);
            if grid > ndp {
                vec![t("ap", "float", 2 * stream * kernels::attn_mma_part_floats(*vd) * 4), counter("ac", items * 4)]
            } else {
                Vec::new()
            }
        }
        // Every head of the group in one block: each chunk's maximum, sum and weighed values, for the fold.
        Launch::Attention { heads, vd, rows, shared: Some(splits), .. } if *splits > 1 => vec![t("ap", "float", rows * heads * splits * (vd + 2) * 4)],
        Launch::Attention { shared: Some(_), .. } => Vec::new(),
        Launch::Attention { heads, vd, rows, per_block, splits, parallel: true, shared: None, .. } if !l.attends_in_tiles(numerics) => {
            vec![t("ap", acc_ty, rows * heads * splits * (vd + 2) * acc), counter("ac", rows * heads / per_block * 4)]
        }
        _ => Vec::new(),
    }
}

fn aligned(n: usize) -> usize {
    n.next_multiple_of(ALIGN)
}

/// The bytes of a tensor of `len` elements' array: its `f32` buffer, or its quantized form — the int8 words, then the
/// low words under two words, then a scale a block — each part aligned.
pub fn array_bytes(len: usize, touch: Touch, two: bool) -> usize {
    match touch {
        Touch::Plain => aligned(len * 4),
        Touch::Quant => aligned(len) * if two { 2 } else { 1 } + aligned(len / QBLOCK * 4),
    }
}

/// Where the parts of a quantized array placed at `offset` start: its words, its low words under two words, its
/// scales.
pub fn quant_parts(offset: usize, len: usize, two: bool) -> (usize, Option<usize>, usize) {
    let words = if two { 2 } else { 1 };
    (offset, two.then(|| offset + aligned(len)), offset + aligned(len) * words)
}

/// How much of an array the step has written before any launch reads it.
#[derive(Clone, Debug)]
enum Written {
    /// All of it.
    Whole,
    /// Parts of every unit of `pitch`, these `[from, to)` so far.
    Parts(usize, Vec<(usize, usize)>),
    /// Read, or written some other way, before it was all written.
    Unsafe,
}

impl Written {
    /// The array after a launch that reads it (`read`), writes it whole (`whole`), writes these parts of every unit
    /// (`parts`), or writes it some other way (`other`).
    fn then(self, read: bool, whole: bool, parts: &[(usize, usize, usize)], other: bool) -> Written {
        let open = match self {
            Written::Parts(pitch, have) => Some((pitch, have)),
            done => return done,
        };
        if read {
            return Written::Unsafe;
        }
        if whole {
            return Written::Whole;
        }
        if other || parts.is_empty() {
            return Written::Unsafe;
        }
        let (pitch, mut have) = open.unwrap_or((parts[0].0, Vec::new()));
        for &(p, from, width) in parts {
            if p != pitch {
                return Written::Unsafe;
            }
            have.push((from, from + width));
        }
        have.sort_unstable();
        let mut reach = 0;
        for &(from, to) in &have {
            if from > reach {
                break;
            }
            reach = reach.max(to);
        }
        if reach >= pitch {
            Written::Whole
        } else {
            Written::Parts(pitch, have)
        }
    }
}

/// Place the arrays of `s` that live within its step: a computed tensor's that is not one of the step's inputs or
/// outputs — or a state the step holds alone (`own`: no other form carries it, so its zeros are only a first step's) —
/// all of it written before any launch reads it — whole, or part by part over every unit (`join_in_place`) — so nothing
/// the step reads of it was left by another array, or by an earlier step. Each lives from its first touch to its last,
/// through any view of it. The largest first, each at the lowest aligned offset clear of every array placed live beside
/// it.
pub fn place(s: &Schedule, own: &dyn Fn(&str) -> bool) -> Arena {
    let two = s.numerics == Q8MatKind::TwoPassInt8;
    let io = s.io.names();
    // A touch through a view is a touch of the tensor it views — of its quantized form too, when the view starts on a
    // whole block and so reads it in place (`home`).
    let viewed = |name: &str, touch: Touch| home(s, name, touch).to_string();
    #[derive(Default)]
    struct Here {
        read: bool,
        whole: bool,
        parts: Vec<(usize, usize, usize)>,
        other: bool,
    }
    let mut life: BTreeMap<(String, Touch), (usize, usize, Written)> = BTreeMap::new();
    for (i, l) in s.launches.iter().enumerate() {
        let (reads, writes) = l.touches(s.numerics);
        let mut here: BTreeMap<(String, Touch), Here> = BTreeMap::new();
        for (t, touch) in reads {
            here.entry((viewed(t, touch), touch)).or_default().read = true;
        }
        for (t, touch) in writes {
            let at = viewed(t, touch);
            let len = s.buffers.get(t).map_or(0, |b| b.len);
            let own = at == t;
            let e = here.entry((at, touch)).or_default();
            if own && l.writes_whole(t, touch, len) {
                e.whole = true;
            } else if let (true, Touch::Plain, Some(part)) = (own, touch, l.writes_part(t, len)) {
                e.parts.push(part);
            } else {
                e.other = true;
            }
        }
        for (k, h) in here {
            let e = life.entry(k).or_insert((i, i, Written::Parts(0, Vec::new())));
            e.1 = i;
            let first = e.0 == i;
            let state = std::mem::replace(&mut e.2, Written::Unsafe);
            e.2 = if first {
                Written::Parts(h.parts.first().map_or(0, |p| p.0), Vec::new()).then(h.read, h.whole, &h.parts, h.other)
            } else {
                state.then(h.read, h.whole, &h.parts, h.other)
            };
        }
    }
    #[derive(PartialEq, Eq, PartialOrd, Ord)]
    enum Item {
        Array(String, Touch),
        Scratch(usize, &'static str),
    }
    let mut todo: Vec<(usize, usize, usize, Item)> = life
        .into_iter()
        .filter(|(_, (.., written))| matches!(written, Written::Whole))
        .filter_map(|((name, touch), (first, last, _))| {
            let b = s.buffers.get(&name)?;
            let allocated = if touch == Touch::Plain { b.plain } else { b.quantized };
            ((b.fill == Fill::Computed || (b.fill == Fill::Zero && own(&name))) && allocated && !io.contains(&name.as_str()))
                .then(|| (array_bytes(b.len, touch, two), first, last, Item::Array(name, touch)))
        })
        .collect();
    for (i, l) in s.launches.iter().enumerate() {
        for sc in scratch(l, s.numerics).into_iter().filter(|sc| !sc.counter) {
            todo.push((aligned(sc.bytes), i, i, Item::Scratch(i, sc.tag)));
        }
    }
    todo.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.3.cmp(&b.3)));
    let mut arena = Arena::default();
    let mut spans: Vec<(usize, usize, usize, usize)> = Vec::new();
    for (bytes, first, last, item) in todo {
        let mut busy: Vec<(usize, usize)> =
            spans.iter().filter(|&&(_, _, f, l)| f <= last && first <= l).map(|&(o, b, _, _)| (o, o + b)).collect();
        busy.sort_unstable();
        let mut offset = 0;
        for (lo, hi) in busy {
            if offset + bytes <= lo {
                break;
            }
            offset = offset.max(hi);
        }
        arena.bytes = arena.bytes.max(offset + bytes);
        spans.push((offset, bytes, first, last));
        match item {
            Item::Array(name, touch) => arena.placed.push(Placed { name, touch, offset, bytes, first, last }),
            Item::Scratch(launch, tag) => arena.scratch.push(PlacedScratch { launch, tag, offset, bytes }),
        }
    }
    arena
}
