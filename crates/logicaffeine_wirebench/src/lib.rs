#![doc = include_str!("../README.md")]

use std::cell::RefCell;
use std::collections::HashMap;
use std::hint::black_box;
use std::rc::Rc;
use std::time::Instant;

use logicaffeine_compile::concurrency::marshal::{
    best_compressed_len, describe_columns, message_from_wire, message_to_wire_with, view_message,
    with_compression_codec, with_floats, with_numerics, with_structure, with_struct_view, WireCodec,
    WireCompression, WireFloats, WireIntegrity, WireNumerics, WireStructure,
};
use logicaffeine_compile::interpreter::{ListRepr, MapStorage, RuntimeValue, StructValue};
use serde::{Deserialize, Serialize};

// ============================================================================
// The structured report (serialized to benchmarks/results/latest-codec.json).
// ============================================================================

/// Which dials the `logos (BEST: all knobs)` row actually selected — surfaced so the page
/// can say *what* won, not just "all knobs". `columns` names the real per-column encoding
/// the codec shipped (from `describe_columns` on the winning bytes): one bare name for a
/// single-column message (`"xor-delta floats"`), or one `"field: encoding"` per field for a
/// record list. `summary` is the preformatted line the card shows verbatim. Present only on
/// the BEST row; every other row names its single dial in its label.
#[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
pub struct ChosenConfig {
    pub numerics: String,
    pub floats: String,
    pub compression: String,
    pub columns: Vec<String>,
    pub summary: String,
}

/// One codec's measured row in a scenario. `read_one_ns` is populated only for the
/// random-access scenario (open the received message + read one field); elsewhere it is
/// `None` and the page ignores it. `chosen` is populated only for the all-knobs winner.
/// `fair_size` is this codec's smallest size when granted the SAME compression LOGOS bakes in
/// (deflate/lz4/zstd, smallest kept) — set on the competitors so the page can show a fair,
/// compressed-vs-compressed size fight, not just LOGOS-compressed-vs-competitor-raw.
#[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
pub struct RowOut {
    pub codec: String,
    pub size: usize,
    pub enc_ns: f64,
    pub dec_ns: f64,
    #[serde(default)]
    pub read_one_ns: Option<f64>,
    #[serde(default)]
    pub chosen: Option<ChosenConfig>,
    #[serde(default)]
    pub fair_size: Option<usize>,
}

/// One workload run through every codec. `kind` partitions the page's rendering:
/// `"fair"` (seeded-random, the headline), `"adversarial"` (shapes others choke on),
/// `"random_access"` (open + read one field — Cap'n Proto's home turf), `"showcase"`
/// (a structured best-case, fenced off from the fair results).
#[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
pub struct CodecScenario {
    pub id: String,
    pub title: String,
    pub n: usize,
    pub kind: String,
    pub rows: Vec<RowOut>,
}

/// Provenance for the run — machine, commit, competitor crate versions, which
/// competitors were compiled in. Filled from env (injected by `benchmarks/run.sh`) with
/// dependency-free fallbacks so a direct local run still produces a complete file.
#[derive(Serialize, Deserialize, PartialEq, Clone, Debug, Default)]
pub struct CodecMetadata {
    pub date: String,
    pub commit: String,
    pub logos_version: String,
    pub cpu: String,
    pub os: String,
    pub versions: HashMap<String, String>,
    pub features: Vec<String>,
}

/// The whole report the page bakes in.
#[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
pub struct CodecReport {
    pub schema_version: u32,
    pub metadata: CodecMetadata,
    pub iters: u32,
    pub scenarios: Vec<CodecScenario>,
}

/// A deterministic SplitMix64 so the "random" payloads are REPRODUCIBLE — a fair
/// benchmark must use random data (a clean `0..n` sequence lets the affine math-hack and
/// tiny-value varints win for free, which is not honest), but it must also be repeatable.
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// A non-negative value with a REALISTIC magnitude spread (ids/counts/sizes): most
    /// values small, a few large — a geometric-ish distribution, not uniform-huge (which
    /// would pathologically favour fixed-width) nor uniform-tiny (which favours varint).
    fn realistic_uint(&mut self) -> i64 {
        let bits = (self.next_u64() % 40) as u32 + 1; // 1..=40 significant bits
        (self.next_u64() & ((1u64 << bits) - 1)) as i64
    }
    /// A SIGNED coordinate-like value (tests the zig-zag path, where protobuf `int64`
    /// pays 10 bytes per negative but our adaptive column does not).
    fn signed_coord(&mut self) -> i64 {
        let m = (self.next_u64() % 200_001) as i64 - 100_000; // [-100000, 100000]
        m
    }
    /// A sensor/coordinate-like f64 in [-1000, 1000] with 3 decimals — realistic mesh /
    /// measurement data (not raw random bits, which would be NaN/inf noise).
    fn realistic_float(&mut self) -> f64 {
        (self.next_u64() % 2_000_000) as f64 / 1000.0 - 1000.0
    }
    fn ascii_string(&mut self, min: usize, max: usize) -> String {
        let len = min + (self.next_u64() as usize % (max - min + 1));
        (0..len).map(|_| (b'a' + (self.next_u64() % 26) as u8) as char).collect()
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq)]
struct Point {
    x: i64,
    y: i64,
}
#[derive(Serialize, Deserialize, Clone, PartialEq)]
struct Record {
    id: i64,
    name: String,
    active: bool,
}

fn rv_list(rows: Vec<RuntimeValue>) -> RuntimeValue {
    RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(rows))))
}

fn point_struct(x: i64, y: i64) -> RuntimeValue {
    let mut f = HashMap::new();
    f.insert("x".to_string(), RuntimeValue::Int(x));
    f.insert("y".to_string(), RuntimeValue::Int(y));
    RuntimeValue::Struct(Box::new(StructValue { type_name: "Point".to_string(), fields: f }))
}
fn record_struct(id: i64, name: &str, active: bool) -> RuntimeValue {
    let mut f = HashMap::new();
    f.insert("id".to_string(), RuntimeValue::Int(id));
    f.insert("name".to_string(), RuntimeValue::Text(Rc::new(name.to_string())));
    f.insert("active".to_string(), RuntimeValue::Bool(active));
    RuntimeValue::Struct(Box::new(StructValue { type_name: "Record".to_string(), fields: f }))
}

/// ns per call: the MIN over 8 batches (each ~`iters`/8 calls under one timer), after a warm-up.
/// Min-of-batches filters scheduler noise on a shared box — a busy run can only make a batch
/// SLOWER, so the minimum is the load-invariant true cost — while keeping total work ≈ `iters`.
/// Both our codec and every competitor are timed identically, so the comparison stays fair.
fn time_ns(iters: u32, mut f: impl FnMut()) -> f64 {
    let per = (iters / 8).max(1);
    for _ in 0..per {
        f(); // warm-up
    }
    let mut best = f64::MAX;
    for _ in 0..8 {
        let t = Instant::now();
        for _ in 0..per {
            f();
        }
        best = best.min(t.elapsed().as_nanos() as f64 / per as f64);
    }
    // Round to whole ns: sub-ns precision on a measurement is noise, and an integer f64 survives
    // a JSON round-trip bit-exactly (repeating decimals like 240927.6666… do not).
    best.round()
}

/// The fastest batch-average ns/op: time a whole batch of `iters` calls with ONE timer
/// (so the per-call `Instant::now()` overhead is amortized, not measured), repeat `rounds`
/// times, and take the MIN batch (filters a scheduler hiccup). Accurate for sub-microsecond
/// ops AND load-robust — the same method the superiority lock-ins use.
fn batch_ns(rounds: u32, iters: u32, mut f: impl FnMut()) -> f64 {
    for _ in 0..iters {
        f();
    }
    let mut best = f64::MAX;
    for _ in 0..rounds {
        let t = Instant::now();
        for _ in 0..iters {
            f();
        }
        best = best.min(t.elapsed().as_nanos() as f64 / iters as f64);
    }
    let best = best.round(); // whole ns — exact JSON round-trip, sub-ns is noise
    best
}

struct Row {
    codec: &'static str,
    size: usize,
    enc_ns: f64,
    dec_ns: f64,
    chosen: Option<ChosenConfig>,
    fair_size: Option<usize>,
}

/// Assemble a scenario from the builder `Row`s, stamping the stable id/kind the page groups by.
fn scenario(id: &str, title: &str, kind: &str, n: usize, rows: Vec<Row>) -> CodecScenario {
    CodecScenario {
        id: id.to_string(),
        title: title.to_string(),
        kind: kind.to_string(),
        n,
        rows: rows
            .into_iter()
            .map(|r| RowOut {
                codec: r.codec.to_string(),
                size: r.size,
                enc_ns: r.enc_ns,
                dec_ns: r.dec_ns,
                read_one_ns: None,
                chosen: r.chosen,
                fair_size: r.fair_size,
            })
            .collect(),
    }
}

/// Run one serde-based competitor: encode (`enc`) + decode (`dec`) the SAME logical
/// data, returning a measured row. `T` is the plain Rust payload type.
fn bench_serde<T, E, D>(codec: &'static str, value: &T, iters: u32, enc: E, dec: D) -> Row
where
    E: Fn(&T) -> Vec<u8>,
    D: Fn(&[u8]) -> T,
{
    let bytes = enc(value);
    let size = bytes.len();
    let enc_ns = time_ns(iters, || {
        black_box(enc(value));
    });
    let dec_ns = time_ns(iters, || {
        black_box(dec(&bytes));
    });
    // Grant this competitor the same compression LOGOS bakes in, so the page can show a FAIR
    // (compressed-vs-compressed) size fight — not just LOGOS-compressed vs this codec raw.
    let fair_size = Some(best_compressed_len(&bytes));
    Row { codec, size, enc_ns, dec_ns, chosen: None, fair_size }
}

/// Our codec on the equivalent `RuntimeValue`, under a chosen numeric strategy.
/// Uses Raw integrity (no checksum) to match the competitors, which carry none —
/// our optional FNV checksum is benched separately as `logos (checked)`.
fn bench_ours_mode(label: &'static str, value: &RuntimeValue, iters: u32, num: WireNumerics) -> Row {
    let enc = || with_numerics(num, || message_to_wire_with("", value, WireCodec::Native, WireIntegrity::Raw).unwrap());
    let bytes = enc();
    let size = bytes.len();
    let enc_ns = time_ns(iters, || {
        black_box(enc());
    });
    let dec_ns = time_ns(iters, || {
        black_box(message_from_wire(&bytes).unwrap());
    });
    Row { codec: label, size, enc_ns, dec_ns, chosen: None, fair_size: None }
}

/// Our codec on the default (Varint) strategy — the smallest-wire default.
fn bench_ours(value: &RuntimeValue, iters: u32) -> Row {
    bench_ours_mode("logos (varint)", value, iters, WireNumerics::Varint)
}

/// Our codec with the *structural* math hack on: integer columns that are exact
/// affine progressions ship as `(base, stride, n)` — the generating formula, not
/// the data. Falls back to Varint when a column is not affine, so it is never worse.
fn bench_ours_affine(value: &RuntimeValue, iters: u32) -> Row {
    let enc = || {
        with_structure(WireStructure::Affine, || {
            with_numerics(WireNumerics::Varint, || {
                message_to_wire_with("", value, WireCodec::Native, WireIntegrity::Raw).unwrap()
            })
        })
    };
    let bytes = enc();
    let size = bytes.len();
    let enc_ns = time_ns(iters, || {
        black_box(enc());
    });
    let dec_ns = time_ns(iters, || {
        black_box(message_from_wire(&bytes).unwrap());
    });
    Row { codec: "logos (affine)", size, enc_ns, dec_ns, chosen: None, fair_size: None }
}

/// "Turn EVERY knob": encode under every dial combination (numerics × structure × floats,
/// each also Zstd-compressed), verify each round-trips, and report the SMALLEST. This is
/// the honest answer to "if we use everything we've got, who still beats us, and where?"
fn bench_ours_best(value: &RuntimeValue, iters: u32) -> Row {
    let encode = |num: WireNumerics, st: WireStructure, fl: WireFloats, comp: WireCompression| -> Vec<u8> {
        let body = || {
            with_numerics(num, || {
                with_structure(st, || {
                    with_floats(fl, || {
                        message_to_wire_with("", value, WireCodec::Native, WireIntegrity::Raw).unwrap()
                    })
                })
            })
        };
        if comp == WireCompression::None {
            body()
        } else {
            with_compression_codec(comp, body)
        }
    };
    // The FULL cross product `message_to_wire_best(Smallest)` searches — numerics × structure
    // (incl. the `Auto` per-column menu: delta/DoD/FOR-bitpack/RLE/dictionary/polynomial) ×
    // float coding × every compressor — so the displayed BEST is the codec's real minimum, not
    // a subset. Each single-dial config is a candidate, so BEST is provably ≤ any single knob.
    let mut cfgs = Vec::new();
    for num in [WireNumerics::Varint, WireNumerics::Fixed, WireNumerics::GroupVarint] {
        for st in [WireStructure::Off, WireStructure::Affine, WireStructure::Auto] {
            for fl in [WireFloats::Memcpy, WireFloats::XorDelta] {
                for comp in
                    [WireCompression::None, WireCompression::Deflate, WireCompression::Lz4, WireCompression::Zstd]
                {
                    cfgs.push((num, st, fl, comp));
                }
            }
        }
    }
    let mut best_i = 0usize;
    let mut best_len = usize::MAX;
    for (i, &(num, st, fl, comp)) in cfgs.iter().enumerate() {
        let b = encode(num, st, fl, comp);
        // Only a configuration that decodes back counts as valid.
        if message_from_wire(&b).is_some() && b.len() < best_len {
            best_len = b.len();
            best_i = i;
        }
    }
    let (num, st, fl, comp) = cfgs[best_i];
    let enc = || encode(num, st, fl, comp);
    let bytes = enc();
    let enc_ns = time_ns(iters, || {
        black_box(enc());
    });
    let dec_ns = time_ns(iters, || {
        black_box(message_from_wire(&bytes).unwrap());
    });
    // Name what actually won. The per-column encodings come from the codec describing its own
    // uncompressed native output (compression wraps the encoding, it doesn't change the column
    // tags), so the card shows the real dials — not just the opaque "all knobs".
    let native = encode(num, st, fl, WireCompression::None);
    let columns = describe_columns(&native);
    let comp_phrase = if comp == WireCompression::None {
        "no compression".to_string()
    } else {
        format!("{}-compressed", compression_word(comp))
    };
    let cols_phrase = if columns.is_empty() { "packed columns".to_string() } else { columns.join(", ") };
    let summary = format!("{cols_phrase} \u{00b7} {comp_phrase}");
    let chosen = ChosenConfig {
        numerics: numerics_word(num).to_string(),
        floats: floats_word(fl).to_string(),
        compression: compression_word(comp).to_string(),
        columns,
        summary,
    };
    // The BEST row already shops compression, so its `size` IS the fair LOGOS size — no separate
    // `fair_size` needed (the page reads BEST.size as the LOGOS side of the fair fight).
    Row { codec: "logos (BEST: all knobs)", size: bytes.len(), enc_ns, dec_ns, chosen: Some(chosen), fair_size: None }
}

/// The plain-words name of each top-level dial, for the winner card's config summary.
fn numerics_word(n: WireNumerics) -> &'static str {
    match n {
        WireNumerics::Varint => "varint",
        WireNumerics::Fixed => "fixed (memcpy)",
        WireNumerics::GroupVarint => "group-varint",
    }
}
fn floats_word(f: WireFloats) -> &'static str {
    match f {
        WireFloats::Memcpy => "memcpy",
        WireFloats::XorDelta => "xor-delta",
    }
}
fn compression_word(c: WireCompression) -> &'static str {
    match c {
        WireCompression::None => "none",
        WireCompression::Deflate => "deflate",
        WireCompression::Lz4 => "lz4",
        WireCompression::Zstd => "zstd",
    }
}

/// A LOGOS row measured on its ZERO-COPY read path — the fair mirror of how Cap'n Proto and Arrow
/// are benched: open a borrowed view over the raw bytes and touch every value IN PLACE, never
/// materializing an owned value. `encode` ships an aligned / view layout (via `with_struct_view`);
/// `read` opens a [`view_message`] and sums every value through the zero-copy slice/field readers.
/// This is the honest apples-to-apples decode: our CONTIGUOUS aligned columns (one SIMD-friendly
/// scan) against their per-element or strided struct reads — beating them at their own game.
fn bench_ours_zerocopy(label: &'static str, encode: impl Fn() -> Vec<u8>, read: impl Fn(&[u8]), iters: u32) -> Row {
    let bytes = encode();
    let size = bytes.len();
    let enc_ns = time_ns(iters, || {
        black_box(encode());
    });
    let dec_ns = time_ns(iters, || {
        read(black_box(&bytes));
    });
    Row { codec: label, size, enc_ns, dec_ns, chosen: None, fair_size: None }
}

/// Encode a value in the zero-copy aligned/view layout (the `with_struct_view` + fixed dial): a
/// plain int or float list becomes an 8-byte-aligned column readable as `&[i64]`/`&[f64]`; a struct
/// list becomes the FIXED-stride record-list view (`T_STRUCTS_FVIEW`) whose cells are read by pure
/// arithmetic + a raw 8-byte load — no offset tables, no per-cell decode. The fixed-layout twin of
/// what Cap'n Proto always emits, so the read comparison is apples-to-apples.
fn encode_zerocopy(rv: &RuntimeValue) -> Vec<u8> {
    with_struct_view(true, || {
        with_numerics(WireNumerics::Fixed, || {
            message_to_wire_with("", rv, WireCodec::Native, WireIntegrity::Raw).unwrap()
        })
    })
}

/// Encode a struct list COLUMNAR with fixed-width columns (`T_STRUCTS` + fixed numerics): every
/// field becomes a contiguous `T_INTS_FIXED` blob. This is LOGOS playing ITS game — columnar, like
/// Arrow — so a bulk read scans each field contiguously instead of striding a row-major layout.
fn encode_columnar(rv: &RuntimeValue) -> Vec<u8> {
    with_numerics(WireNumerics::Fixed, || message_to_wire_with("", rv, WireCodec::Native, WireIntegrity::Raw).unwrap())
}

/// Zero-copy sum of a columnar two-int-field struct list (points {x, y}): read each field's
/// CONTIGUOUS blob in place and scan it. All x's are adjacent, then all y's — two cache-friendly
/// linear passes that the compiler vectorizes — beating Cap'n Proto's INTERLEAVED (x0,y0,x1,y1…)
/// row-major read where summing one field strides the whole record every step.
fn zerocopy_sum_points(bytes: &[u8]) -> i64 {
    let Some(view) = view_message(bytes) else { return 0 };
    let mut s = 0i64;
    // Fields are stored in canonical (sorted) order: {x, y} → column 0 = x, column 1 = y.
    for fi in 0..2 {
        if let Some(blob) = view.structs_fixed_i64_col(fi) {
            for chunk in blob.chunks_exact(8) {
                s = s.wrapping_add(i64::from_le_bytes(chunk.try_into().unwrap()));
            }
        }
    }
    s
}

/// Sum every i64 of an aligned column IN PLACE (zero-copy) — a single contiguous SIMD-friendly scan
/// over the borrowed bytes, no materialization. Falls back to a decode only if the buffer is not
/// 8-aligned in memory (a fresh aligned encode on a 16-aligned allocation never hits this); the
/// fallback keeps the result correct, and `zerocopy_reads_are_in_place` asserts the fast path fires.
fn zerocopy_sum_i64(bytes: &[u8]) -> i64 {
    if let Some(slice) = view_message(bytes).and_then(|v| v.as_i64_slice()) {
        return slice.iter().fold(0i64, |a, &x| a.wrapping_add(x));
    }
    match message_from_wire(bytes).map(|m| m.1) {
        Some(RuntimeValue::List(l)) => match &*l.borrow() {
            ListRepr::Ints(g) => g.iter().fold(0i64, |a, &x| a.wrapping_add(x)),
            _ => 0,
        },
        _ => 0,
    }
}

/// The float twin of [`zerocopy_sum_i64`]: sum every f64 of an aligned column in place.
fn zerocopy_sum_f64(bytes: &[u8]) -> f64 {
    if let Some(slice) = view_message(bytes).and_then(|v| v.as_f64_slice()) {
        return slice.iter().fold(0f64, |a, &x| a + x);
    }
    match message_from_wire(bytes).map(|m| m.1) {
        Some(RuntimeValue::List(l)) => match &*l.borrow() {
            ListRepr::Floats(g) => g.iter().fold(0f64, |a, &x| a + x),
            _ => 0.0,
        },
        _ => 0.0,
    }
}

/// A codec given purely as opaque-blob encode/decode closures (for non-serde
/// competitors like Arrow). `dec` must *touch every value* it reads — the honest
/// measure of "decode to something usable", so a format that decodes lazily is
/// credited for exactly the work it actually does.
#[allow(dead_code)]
fn bench_blob(codec: &'static str, iters: u32, enc: impl Fn() -> Vec<u8>, dec: impl Fn(&[u8])) -> Row {
    let bytes = enc();
    let size = bytes.len();
    let enc_ns = time_ns(iters, || {
        black_box(enc());
    });
    let dec_ns = time_ns(iters, || {
        dec(black_box(&bytes));
    });
    let fair_size = Some(best_compressed_len(&bytes));
    Row { codec, size, enc_ns, dec_ns, chosen: None, fair_size }
}

/// Arrow rows are produced only under `--features arrow-bench`; otherwise these
/// return nothing, so the call sites stay identical and warning-free.
#[cfg(feature = "arrow-bench")]
mod arrow_bench {
    use super::{bench_blob, Point, Record, Row};
    use std::hint::black_box;
    use std::sync::Arc;

    use arrow::array::{Array, BooleanArray, Float64Array, Int64Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::ipc::reader::StreamReader;
    use arrow::ipc::writer::StreamWriter;
    use arrow::record_batch::RecordBatch;

    fn ipc_encode(batch: &RecordBatch) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut w = StreamWriter::try_new(&mut buf, &batch.schema()).unwrap();
            w.write(batch).unwrap();
            w.finish().unwrap();
        }
        buf
    }

    fn batch(fields: Vec<Field>, cols: Vec<arrow::array::ArrayRef>) -> RecordBatch {
        let schema = Arc::new(Schema::new(fields));
        RecordBatch::try_new(schema, cols).unwrap()
    }

    pub fn ints(plain: &[i64], iters: u32) -> Row {
        let b = batch(
            vec![Field::new("v", DataType::Int64, false)],
            vec![Arc::new(Int64Array::from(plain.to_vec()))],
        );
        bench_blob("arrow (ipc)", iters, || ipc_encode(&b), |bytes| {
            let reader = StreamReader::try_new(bytes, None).unwrap();
            let mut sum = 0i64;
            for rb in reader {
                let rb = rb.unwrap();
                let c = rb.column(0).as_any().downcast_ref::<Int64Array>().unwrap();
                for i in 0..c.len() {
                    sum = sum.wrapping_add(c.value(i));
                }
            }
            black_box(sum);
        })
    }

    pub fn floats(plain: &[f64], iters: u32) -> Row {
        let b = batch(
            vec![Field::new("v", DataType::Float64, false)],
            vec![Arc::new(Float64Array::from(plain.to_vec()))],
        );
        bench_blob("arrow (ipc)", iters, || ipc_encode(&b), |bytes| {
            let reader = StreamReader::try_new(bytes, None).unwrap();
            let mut sum = 0f64;
            for rb in reader {
                let rb = rb.unwrap();
                let c = rb.column(0).as_any().downcast_ref::<Float64Array>().unwrap();
                for i in 0..c.len() {
                    sum += c.value(i);
                }
            }
            black_box(sum);
        })
    }

    pub fn points(plain: &[Point], iters: u32) -> Row {
        let xs: Vec<i64> = plain.iter().map(|p| p.x).collect();
        let ys: Vec<i64> = plain.iter().map(|p| p.y).collect();
        let b = batch(
            vec![Field::new("x", DataType::Int64, false), Field::new("y", DataType::Int64, false)],
            vec![Arc::new(Int64Array::from(xs)), Arc::new(Int64Array::from(ys))],
        );
        bench_blob("arrow (ipc)", iters, || ipc_encode(&b), |bytes| {
            let reader = StreamReader::try_new(bytes, None).unwrap();
            let mut sum = 0i64;
            for rb in reader {
                let rb = rb.unwrap();
                let x = rb.column(0).as_any().downcast_ref::<Int64Array>().unwrap();
                let y = rb.column(1).as_any().downcast_ref::<Int64Array>().unwrap();
                for i in 0..x.len() {
                    sum = sum.wrapping_add(x.value(i)).wrapping_add(y.value(i));
                }
            }
            black_box(sum);
        })
    }

    pub fn records(plain: &[Record], iters: u32) -> Row {
        let ids: Vec<i64> = plain.iter().map(|r| r.id).collect();
        let names = StringArray::from_iter_values(plain.iter().map(|r| r.name.as_str()));
        let actives: Vec<bool> = plain.iter().map(|r| r.active).collect();
        let b = batch(
            vec![
                Field::new("id", DataType::Int64, false),
                Field::new("name", DataType::Utf8, false),
                Field::new("active", DataType::Boolean, false),
            ],
            vec![Arc::new(Int64Array::from(ids)), Arc::new(names), Arc::new(BooleanArray::from(actives))],
        );
        bench_blob("arrow (ipc)", iters, || ipc_encode(&b), |bytes| {
            let reader = StreamReader::try_new(bytes, None).unwrap();
            let mut acc = 0i64;
            for rb in reader {
                let rb = rb.unwrap();
                let id = rb.column(0).as_any().downcast_ref::<Int64Array>().unwrap();
                let name = rb.column(1).as_any().downcast_ref::<StringArray>().unwrap();
                let act = rb.column(2).as_any().downcast_ref::<BooleanArray>().unwrap();
                for i in 0..id.len() {
                    acc = acc.wrapping_add(id.value(i)).wrapping_add(name.value(i).len() as i64);
                    acc = acc.wrapping_add(act.value(i) as i64);
                }
            }
            black_box(acc);
        })
    }

    pub fn strings(plain: &[String], iters: u32) -> Row {
        let arr = StringArray::from_iter_values(plain.iter().map(|s| s.as_str()));
        let b = batch(vec![Field::new("v", DataType::Utf8, false)], vec![Arc::new(arr)]);
        bench_blob("arrow (ipc)", iters, || ipc_encode(&b), |bytes| {
            let reader = StreamReader::try_new(bytes, None).unwrap();
            let mut acc = 0i64;
            for rb in reader {
                let rb = rb.unwrap();
                let c = rb.column(0).as_any().downcast_ref::<StringArray>().unwrap();
                for i in 0..c.len() {
                    acc = acc.wrapping_add(c.value(i).len() as i64);
                }
            }
            black_box(acc);
        })
    }

    /// Open the IPC stream and read ONE field of one row — Arrow must materialize the batch
    /// columns to index, the honest cost of a single-field read for a batch-columnar format.
    pub fn record_read_one(bytes: &[u8], i: usize) {
        let reader = StreamReader::try_new(bytes, None).unwrap();
        let mut got = 0i64;
        for rb in reader {
            let rb = rb.unwrap();
            let id = rb.column(0).as_any().downcast_ref::<Int64Array>().unwrap();
            if i < id.len() {
                got = id.value(i);
                break;
            }
        }
        black_box(got);
    }

    pub fn encode_records(plain: &[Record]) -> Vec<u8> {
        let ids: Vec<i64> = plain.iter().map(|r| r.id).collect();
        let names = StringArray::from_iter_values(plain.iter().map(|r| r.name.as_str()));
        let actives: Vec<bool> = plain.iter().map(|r| r.active).collect();
        let b = batch(
            vec![
                Field::new("id", DataType::Int64, false),
                Field::new("name", DataType::Utf8, false),
                Field::new("active", DataType::Boolean, false),
            ],
            vec![Arc::new(Int64Array::from(ids)), Arc::new(names), Arc::new(BooleanArray::from(actives))],
        );
        ipc_encode(&b)
    }

    pub fn record_decode_all(bytes: &[u8]) {
        let reader = StreamReader::try_new(bytes, None).unwrap();
        let mut acc = 0i64;
        for rb in reader {
            let rb = rb.unwrap();
            let id = rb.column(0).as_any().downcast_ref::<Int64Array>().unwrap();
            for i in 0..id.len() {
                acc = acc.wrapping_add(id.value(i));
            }
        }
        black_box(acc);
    }
}

#[cfg(feature = "arrow-bench")]
fn arrow_int_rows(plain: &[i64], iters: u32) -> Vec<Row> {
    vec![arrow_bench::ints(plain, iters)]
}
#[cfg(not(feature = "arrow-bench"))]
fn arrow_int_rows(_: &[i64], _: u32) -> Vec<Row> {
    Vec::new()
}

#[cfg(feature = "arrow-bench")]
fn arrow_float_rows(plain: &[f64], iters: u32) -> Vec<Row> {
    vec![arrow_bench::floats(plain, iters)]
}
#[cfg(not(feature = "arrow-bench"))]
fn arrow_float_rows(_: &[f64], _: u32) -> Vec<Row> {
    Vec::new()
}

#[cfg(feature = "arrow-bench")]
fn arrow_point_rows(plain: &[Point], iters: u32) -> Vec<Row> {
    vec![arrow_bench::points(plain, iters)]
}
#[cfg(not(feature = "arrow-bench"))]
fn arrow_point_rows(_: &[Point], _: u32) -> Vec<Row> {
    Vec::new()
}

#[cfg(feature = "arrow-bench")]
fn arrow_record_rows(plain: &[Record], iters: u32) -> Vec<Row> {
    vec![arrow_bench::records(plain, iters)]
}
#[cfg(not(feature = "arrow-bench"))]
fn arrow_record_rows(_: &[Record], _: u32) -> Vec<Row> {
    Vec::new()
}

#[cfg(feature = "arrow-bench")]
fn arrow_string_rows(plain: &[String], iters: u32) -> Vec<Row> {
    vec![arrow_bench::strings(plain, iters)]
}
#[cfg(not(feature = "arrow-bench"))]
fn arrow_string_rows(_: &[String], _: u32) -> Vec<Row> {
    Vec::new()
}

#[cfg(feature = "arrow-bench")]
fn arrow_ra_rows(plain: &[Record], idxs: &[usize], iters: u32) -> Vec<RowOut> {
    let plain = plain.to_vec();
    vec![ra_row(
        "arrow (ipc)",
        iters,
        idxs,
        move || arrow_bench::encode_records(&plain),
        arrow_bench::record_decode_all,
        arrow_bench::record_read_one,
    )]
}
#[cfg(not(feature = "arrow-bench"))]
fn arrow_ra_rows(_: &[Record], _: &[usize], _: u32) -> Vec<RowOut> {
    Vec::new()
}

// ---- protobuf (also the gRPC payload codec — HTTP/2 is just the transport) ----

#[cfg(feature = "protobuf")]
mod proto {
    include!(concat!(env!("OUT_DIR"), "/bench.rs"));
}

#[cfg(feature = "protobuf")]
fn proto_int_rows(plain: &[i64], iters: u32) -> Vec<Row> {
    use prost::Message;
    let msg = proto::Ints { v: plain.to_vec() };
    vec![bench_blob("protobuf/grpc", iters, || msg.encode_to_vec(), |bytes| {
        let m = proto::Ints::decode(bytes).unwrap();
        let mut s = 0i64;
        for x in &m.v {
            s = s.wrapping_add(*x);
        }
        std::hint::black_box(s);
    })]
}
#[cfg(not(feature = "protobuf"))]
fn proto_int_rows(_: &[i64], _: u32) -> Vec<Row> {
    Vec::new()
}

#[cfg(feature = "protobuf")]
fn proto_point_rows(plain: &[Point], iters: u32) -> Vec<Row> {
    use prost::Message;
    let msg = proto::Points { items: plain.iter().map(|p| proto::Point { x: p.x, y: p.y }).collect() };
    vec![bench_blob("protobuf/grpc", iters, || msg.encode_to_vec(), |bytes| {
        let m = proto::Points::decode(bytes).unwrap();
        let mut s = 0i64;
        for p in &m.items {
            s = s.wrapping_add(p.x).wrapping_add(p.y);
        }
        std::hint::black_box(s);
    })]
}
#[cfg(not(feature = "protobuf"))]
fn proto_point_rows(_: &[Point], _: u32) -> Vec<Row> {
    Vec::new()
}

#[cfg(feature = "protobuf")]
fn proto_record_rows(plain: &[Record], iters: u32) -> Vec<Row> {
    use prost::Message;
    let msg = proto::Records {
        items: plain.iter().map(|r| proto::Record { id: r.id, name: r.name.clone(), active: r.active }).collect(),
    };
    vec![bench_blob("protobuf/grpc", iters, || msg.encode_to_vec(), |bytes| {
        let m = proto::Records::decode(bytes).unwrap();
        let mut acc = 0i64;
        for r in &m.items {
            acc = acc.wrapping_add(r.id).wrapping_add(r.name.len() as i64).wrapping_add(r.active as i64);
        }
        std::hint::black_box(acc);
    })]
}
#[cfg(not(feature = "protobuf"))]
fn proto_record_rows(_: &[Record], _: u32) -> Vec<Row> {
    Vec::new()
}

#[cfg(feature = "protobuf")]
fn proto_string_rows(plain: &[String], iters: u32) -> Vec<Row> {
    use prost::Message;
    let msg = proto::Strings { v: plain.to_vec() };
    vec![bench_blob("protobuf/grpc", iters, || msg.encode_to_vec(), |bytes| {
        let m = proto::Strings::decode(bytes).unwrap();
        let mut acc = 0i64;
        for s in &m.v {
            acc = acc.wrapping_add(s.len() as i64);
        }
        std::hint::black_box(acc);
    })]
}
#[cfg(not(feature = "protobuf"))]
fn proto_string_rows(_: &[String], _: u32) -> Vec<Row> {
    Vec::new()
}

#[cfg(feature = "protobuf")]
fn proto_ra_rows(plain: &[Record], idxs: &[usize], iters: u32) -> Vec<RowOut> {
    use prost::Message;
    let msg = proto::Records {
        items: plain.iter().map(|r| proto::Record { id: r.id, name: r.name.clone(), active: r.active }).collect(),
    };
    // protobuf is length-delimited, not random-access: reading one field means decoding the
    // whole message and indexing — the honest cost for this format.
    vec![ra_row(
        "protobuf/grpc",
        iters,
        idxs,
        move || msg.encode_to_vec(),
        |bytes| {
            let m = proto::Records::decode(bytes).unwrap();
            std::hint::black_box(m.items.len());
        },
        |bytes, i| {
            let m = proto::Records::decode(bytes).unwrap();
            std::hint::black_box(m.items[i].id);
        },
    )]
}
#[cfg(not(feature = "protobuf"))]
fn proto_ra_rows(_: &[Record], _: &[usize], _: u32) -> Vec<RowOut> {
    Vec::new()
}

// ---- Cap'n Proto (zero-copy: decode reads straight from the flat buffer) ----

#[cfg(feature = "capnproto")]
mod bench_capnp {
    include!(concat!(env!("OUT_DIR"), "/schemas/bench_capnp.rs"));
}

// Read a Cap'n Proto message ZERO-COPY: `read_message_from_flat_slice` borrows the
// buffer directly (no segment copy), which is the format's whole point. Using the
// owned-segments `read_message` here would charge capnp a memcpy it is designed to
// avoid — measuring it at its best keeps our comparison honest.
#[cfg(feature = "capnproto")]
macro_rules! capnp_reader {
    ($bytes:expr) => {{
        let mut slice: &[u8] = $bytes;
        capnp::serialize::read_message_from_flat_slice(&mut slice, capnp::message::ReaderOptions::new()).unwrap()
    }};
}

#[cfg(feature = "capnproto")]
fn capnp_int_rows(plain: &[i64], iters: u32) -> Vec<Row> {
    let plain = plain.to_vec();
    vec![bench_blob(
        "capnproto",
        iters,
        || {
            let mut b = capnp::message::Builder::new_default();
            {
                let root = b.init_root::<bench_capnp::ints::Builder>();
                let mut list = root.init_v(plain.len() as u32);
                for (i, x) in plain.iter().enumerate() {
                    list.set(i as u32, *x);
                }
            }
            let mut buf = Vec::new();
            capnp::serialize::write_message(&mut buf, &b).unwrap();
            buf
        },
        |bytes| {
            let reader = capnp_reader!(bytes);
            let root = reader.get_root::<bench_capnp::ints::Reader>().unwrap();
            let list = root.get_v().unwrap();
            let mut s = 0i64;
            for i in 0..list.len() {
                s = s.wrapping_add(list.get(i));
            }
            std::hint::black_box(s);
        },
    )]
}
#[cfg(not(feature = "capnproto"))]
fn capnp_int_rows(_: &[i64], _: u32) -> Vec<Row> {
    Vec::new()
}

#[cfg(feature = "capnproto")]
fn capnp_point_rows(plain: &[Point], iters: u32) -> Vec<Row> {
    let plain = plain.to_vec();
    vec![bench_blob(
        "capnproto",
        iters,
        || {
            let mut b = capnp::message::Builder::new_default();
            {
                let root = b.init_root::<bench_capnp::points::Builder>();
                let mut items = root.init_items(plain.len() as u32);
                for (i, p) in plain.iter().enumerate() {
                    let mut pb = items.reborrow().get(i as u32);
                    pb.set_x(p.x);
                    pb.set_y(p.y);
                }
            }
            let mut buf = Vec::new();
            capnp::serialize::write_message(&mut buf, &b).unwrap();
            buf
        },
        |bytes| {
            let reader = capnp_reader!(bytes);
            let root = reader.get_root::<bench_capnp::points::Reader>().unwrap();
            let items = root.get_items().unwrap();
            let mut s = 0i64;
            for i in 0..items.len() {
                let p = items.get(i);
                s = s.wrapping_add(p.get_x()).wrapping_add(p.get_y());
            }
            std::hint::black_box(s);
        },
    )]
}
#[cfg(not(feature = "capnproto"))]
fn capnp_point_rows(_: &[Point], _: u32) -> Vec<Row> {
    Vec::new()
}

#[cfg(feature = "capnproto")]
fn capnp_record_rows(plain: &[Record], iters: u32) -> Vec<Row> {
    let plain = plain.to_vec();
    vec![bench_blob(
        "capnproto",
        iters,
        || {
            let mut b = capnp::message::Builder::new_default();
            {
                let root = b.init_root::<bench_capnp::records::Builder>();
                let mut items = root.init_items(plain.len() as u32);
                for (i, r) in plain.iter().enumerate() {
                    let mut rb = items.reborrow().get(i as u32);
                    rb.set_id(r.id);
                    rb.set_name(r.name.as_str());
                    rb.set_active(r.active);
                }
            }
            let mut buf = Vec::new();
            capnp::serialize::write_message(&mut buf, &b).unwrap();
            buf
        },
        |bytes| {
            let reader = capnp_reader!(bytes);
            let root = reader.get_root::<bench_capnp::records::Reader>().unwrap();
            let items = root.get_items().unwrap();
            let mut acc = 0i64;
            for i in 0..items.len() {
                let r = items.get(i);
                let name = r.get_name().unwrap();
                acc = acc.wrapping_add(r.get_id()).wrapping_add(name.len() as i64).wrapping_add(r.get_active() as i64);
            }
            std::hint::black_box(acc);
        },
    )]
}
#[cfg(not(feature = "capnproto"))]
fn capnp_record_rows(_: &[Record], _: u32) -> Vec<Row> {
    Vec::new()
}

#[cfg(feature = "capnproto")]
fn capnp_string_rows(plain: &[String], iters: u32) -> Vec<Row> {
    let plain = plain.to_vec();
    vec![bench_blob(
        "capnproto",
        iters,
        || {
            let mut b = capnp::message::Builder::new_default();
            {
                let root = b.init_root::<bench_capnp::strings::Builder>();
                let mut list = root.init_v(plain.len() as u32);
                for (i, s) in plain.iter().enumerate() {
                    list.set(i as u32, s.as_str());
                }
            }
            let mut buf = Vec::new();
            capnp::serialize::write_message(&mut buf, &b).unwrap();
            buf
        },
        |bytes| {
            let reader = capnp_reader!(bytes);
            let root = reader.get_root::<bench_capnp::strings::Reader>().unwrap();
            let list = root.get_v().unwrap();
            let mut acc = 0i64;
            for i in 0..list.len() {
                let s = list.get(i).unwrap();
                acc = acc.wrapping_add(s.len() as i64);
            }
            std::hint::black_box(acc);
        },
    )]
}
#[cfg(not(feature = "capnproto"))]
fn capnp_string_rows(_: &[String], _: u32) -> Vec<Row> {
    Vec::new()
}

#[cfg(feature = "capnproto")]
fn capnp_ra_rows(plain: &[Record], idxs: &[usize], iters: u32) -> Vec<RowOut> {
    let plain = plain.to_vec();
    let build = move || {
        let mut b = capnp::message::Builder::new_default();
        {
            let root = b.init_root::<bench_capnp::records::Builder>();
            let mut items = root.init_items(plain.len() as u32);
            for (i, r) in plain.iter().enumerate() {
                let mut rb = items.reborrow().get(i as u32);
                rb.set_id(r.id);
                rb.set_name(r.name.as_str());
                rb.set_active(r.active);
            }
        }
        let mut buf = Vec::new();
        capnp::serialize::write_message(&mut buf, &b).unwrap();
        buf
    };
    vec![ra_row(
        "capnproto",
        iters,
        idxs,
        build,
        |bytes| {
            let reader = capnp_reader!(bytes);
            let items = reader.get_root::<bench_capnp::records::Reader>().unwrap().get_items().unwrap();
            let mut acc = 0i64;
            for i in 0..items.len() {
                acc = acc.wrapping_add(items.get(i).get_id());
            }
            std::hint::black_box(acc);
        },
        |bytes, i| {
            let reader = capnp_reader!(bytes);
            let items = reader.get_root::<bench_capnp::records::Reader>().unwrap().get_items().unwrap();
            std::hint::black_box(items.get(i as u32).get_id());
        },
    )]
}
#[cfg(not(feature = "capnproto"))]
fn capnp_ra_rows(_: &[Record], _: &[usize], _: u32) -> Vec<RowOut> {
    Vec::new()
}

fn print_table(s: &CodecScenario) {
    let json = s.rows.iter().find(|r| r.codec == "json").map_or(1, |r| r.size.max(1));
    let has_read = s.rows.iter().any(|r| r.read_one_ns.is_some());
    println!("\n=== {} (n={}) ===", s.title, s.n);
    if has_read {
        println!(
            "  {:<16} {:>10} {:>12} {:>12} {:>10} {:>13}",
            "codec", "size B", "enc ns/op", "dec ns/op", "× json", "read1 ns/op"
        );
    } else {
        println!("  {:<16} {:>10} {:>12} {:>12} {:>10}", "codec", "size B", "enc ns/op", "dec ns/op", "× json");
    }
    for r in &s.rows {
        if has_read {
            let read = r.read_one_ns.map(|v| format!("{v:.0}")).unwrap_or_else(|| "—".to_string());
            println!(
                "  {:<16} {:>10} {:>12.0} {:>12.0} {:>9.1}× {:>13}",
                r.codec,
                r.size,
                r.enc_ns,
                r.dec_ns,
                json as f64 / r.size.max(1) as f64,
                read
            );
        } else {
            println!(
                "  {:<16} {:>10} {:>12.0} {:>12.0} {:>9.1}×",
                r.codec,
                r.size,
                r.enc_ns,
                r.dec_ns,
                json as f64 / r.size.max(1) as f64
            );
        }
    }
}

fn bench_points(n: usize, iters: u32) -> CodecScenario {
    // Random SIGNED coordinates (the zig-zag path) — where protobuf `int64` pays 10 bytes
    // per negative, but our adaptive column spends ~3. Independent x and y (not y = 2x).
    let mut rng = Rng::new(0xB2_B2_B2);
    let coords: Vec<(i64, i64)> = (0..n).map(|_| (rng.signed_coord(), rng.signed_coord())).collect();
    let plain: Vec<Point> = coords.iter().map(|&(x, y)| Point { x, y }).collect();
    let rv = rv_list(coords.iter().map(|&(x, y)| point_struct(x, y)).collect());
    let mut rows = vec![
        bench_ours(&rv, iters),
        bench_ours_best(&rv, iters),
        bench_ours_mode("logos (fixed)", &rv, iters, WireNumerics::Fixed),
        bench_ours_mode("logos (gv/simd)", &rv, iters, WireNumerics::GroupVarint),
        bench_ours_affine(&rv, iters),
        // Zero-copy read: columnar fixed layout — each field a contiguous blob, scanned in place.
        // Beats Cap'n Proto's interleaved row-major read on a bulk sum (columnar-analytics win).
        bench_ours_zerocopy("logos (zero-copy)", || encode_columnar(&rv), |b| { black_box(zerocopy_sum_points(b)); }, iters),
        bench_serde("bincode", &plain, iters, |v| bincode::serialize(v).unwrap(), |b| bincode::deserialize(b).unwrap()),
        bench_serde("postcard", &plain, iters, |v| postcard::to_allocvec(v).unwrap(), |b| postcard::from_bytes(b).unwrap()),
        bench_serde("messagepack", &plain, iters, |v| rmp_serde::to_vec(v).unwrap(), |b| rmp_serde::from_slice(b).unwrap()),
        bench_serde(
            "cbor",
            &plain,
            iters,
            |v| {
                let mut o = Vec::new();
                ciborium::into_writer(v, &mut o).unwrap();
                o
            },
            |b| ciborium::from_reader(b).unwrap(),
        ),
        bench_serde("json", &plain, iters, |v| serde_json::to_vec(v).unwrap(), |b| serde_json::from_slice(b).unwrap()),
    ];
    rows.extend(arrow_point_rows(&plain, iters));
    rows.extend(proto_point_rows(&plain, iters));
    rows.extend(capnp_point_rows(&plain, iters));
    scenario("points", "Point list {x:i64,y:i64}", "fair", n, rows)
}

fn bench_records(n: usize, iters: u32) -> CodecScenario {
    // Random records: realistic-id + random short name + random flag — a `log`-row-like
    // mixed-type struct (the shape the standard rust_serialization_benchmark `log` set uses).
    let mut rng = Rng::new(0xC3_C3_C3);
    let data: Vec<(i64, String, bool)> = (0..n)
        .map(|_| (rng.realistic_uint(), rng.ascii_string(4, 12), rng.next_u64() & 1 == 0))
        .collect();
    let plain: Vec<Record> =
        data.iter().map(|(id, name, active)| Record { id: *id, name: name.clone(), active: *active }).collect();
    let rv = rv_list(data.iter().map(|(id, name, active)| record_struct(*id, name, *active)).collect());
    let mut rows = vec![
        bench_ours(&rv, iters),
        bench_ours_best(&rv, iters),
        bench_ours_mode("logos (fixed)", &rv, iters, WireNumerics::Fixed),
        bench_serde("bincode", &plain, iters, |v| bincode::serialize(v).unwrap(), |b| bincode::deserialize(b).unwrap()),
        bench_serde("postcard", &plain, iters, |v| postcard::to_allocvec(v).unwrap(), |b| postcard::from_bytes(b).unwrap()),
        bench_serde("messagepack", &plain, iters, |v| rmp_serde::to_vec(v).unwrap(), |b| rmp_serde::from_slice(b).unwrap()),
        bench_serde(
            "cbor",
            &plain,
            iters,
            |v| {
                let mut o = Vec::new();
                ciborium::into_writer(v, &mut o).unwrap();
                o
            },
            |b| ciborium::from_reader(b).unwrap(),
        ),
        bench_serde("json", &plain, iters, |v| serde_json::to_vec(v).unwrap(), |b| serde_json::from_slice(b).unwrap()),
    ];
    rows.extend(arrow_record_rows(&plain, iters));
    rows.extend(proto_record_rows(&plain, iters));
    rows.extend(capnp_record_rows(&plain, iters));
    scenario("records", "Record list {id,name,active}", "fair", n, rows)
}

/// Run one int-array scenario (the SAME logical data through every codec, all our knobs
/// turned) under a descriptive title — reused for the fair random case and the
/// adversarial "codecs choke here" cases.
fn bench_int_scenario(id: &str, title: &str, kind: &str, plain: Vec<i64>, iters: u32) -> CodecScenario {
    let n = plain.len();
    let rv = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(plain.clone()))));
    let mut rows = vec![
        bench_ours(&rv, iters),
        bench_ours_best(&rv, iters),
        bench_ours_mode("logos (fixed)", &rv, iters, WireNumerics::Fixed),
        bench_ours_mode("logos (gv/simd)", &rv, iters, WireNumerics::GroupVarint),
        bench_ours_affine(&rv, iters),
        // Zero-copy read: an 8-byte-aligned column summed in place — the fair mirror of capnp/arrow.
        bench_ours_zerocopy("logos (zero-copy)", || encode_zerocopy(&rv), |b| { black_box(zerocopy_sum_i64(b)); }, iters),
        bench_serde("bincode", &plain, iters, |v| bincode::serialize(v).unwrap(), |b| bincode::deserialize(b).unwrap()),
        bench_serde("postcard", &plain, iters, |v| postcard::to_allocvec(v).unwrap(), |b| postcard::from_bytes(b).unwrap()),
        bench_serde("messagepack", &plain, iters, |v| rmp_serde::to_vec(v).unwrap(), |b| rmp_serde::from_slice(b).unwrap()),
        bench_serde("json", &plain, iters, |v| serde_json::to_vec(v).unwrap(), |b| serde_json::from_slice(b).unwrap()),
    ];
    rows.extend(arrow_int_rows(&plain, iters));
    rows.extend(proto_int_rows(&plain, iters));
    rows.extend(capnp_int_rows(&plain, iters));
    scenario(id, title, kind, n, rows)
}

fn bench_ints(n: usize, iters: u32) -> CodecScenario {
    // Random non-negative ints with a realistic magnitude spread (ids/counts/sizes) —
    // NOT a `0..n` sequence (which the affine hack would elide to 7 bytes for free).
    let mut rng = Rng::new(0xA1_A1_A1);
    let plain: Vec<i64> = (0..n).map(|_| rng.realistic_uint()).collect();
    bench_int_scenario("ints", "Int array (random)", "fair", plain, iters)
}

/// Float race: random sensor/mesh-like f64. Our float column is memcpy by default, with a
/// Gorilla XOR-delta knob the `BEST` row tries. Competitors: serde codecs + arrow Float64.
fn bench_floats(n: usize, iters: u32) -> CodecScenario {
    let mut rng = Rng::new(0xF0_F0_F0);
    let plain: Vec<f64> = (0..n).map(|_| rng.realistic_float()).collect();
    let rv = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(plain.clone()))));
    let mut rows = vec![
        bench_ours(&rv, iters),
        bench_ours_best(&rv, iters),
        bench_ours_mode("logos (fixed)", &rv, iters, WireNumerics::Fixed),
        // Zero-copy read: an 8-byte-aligned f64 column summed in place — the fair mirror of arrow.
        bench_ours_zerocopy("logos (zero-copy)", || encode_zerocopy(&rv), |b| { black_box(zerocopy_sum_f64(b)); }, iters),
        bench_serde("bincode", &plain, iters, |v| bincode::serialize(v).unwrap(), |b| bincode::deserialize(b).unwrap()),
        bench_serde("postcard", &plain, iters, |v| postcard::to_allocvec(v).unwrap(), |b| postcard::from_bytes(b).unwrap()),
        bench_serde("messagepack", &plain, iters, |v| rmp_serde::to_vec(v).unwrap(), |b| rmp_serde::from_slice(b).unwrap()),
        bench_serde("json", &plain, iters, |v| serde_json::to_vec(v).unwrap(), |b| serde_json::from_slice(b).unwrap()),
    ];
    rows.extend(arrow_float_rows(&plain, iters));
    scenario("floats", "Float array (random f64)", "fair", n, rows)
}

/// Adversarial scenarios — the data shapes other codecs choke on, with all our knobs on.
fn bench_adversarial(iters: u32) -> Vec<CodecScenario> {
    let mut rng = Rng::new(0xE5_E5_E5);
    // 1. ALL NEGATIVE: protobuf `int64` spends 10 bytes per negative; our adaptive
    //    zig-zag spends 1–3. The scenario protobuf is worst at.
    let neg: Vec<i64> = (0..1000).map(|_| -(rng.realistic_uint().max(1))).collect();
    let s1 = bench_int_scenario(
        "adv_negative",
        "Int array — ALL NEGATIVE (protobuf int64 = 10B/elem)",
        "adversarial",
        neg,
        iters,
    );
    // 2. REPETITIVE: a tiny value set cycled — compression/dictionary territory; the
    //    BEST (all-knobs) row turns on Zstd, the fixed-width codecs cannot shrink.
    let rep: Vec<i64> = (0..2000).map(|i| ((i % 8) as i64) * 1_000_000).collect();
    let s2 = bench_int_scenario(
        "adv_repetitive",
        "Int array — REPETITIVE (compression shines; fixed codecs can't)",
        "adversarial",
        rep,
        iters,
    );
    // 3. HUGE MAGNITUDE: full 64-bit random — varint loses to fixed; the dial lets us
    //    pick memcpy (8B, fast) where varint-only codecs (protobuf) stay at ~9–10B.
    let huge: Vec<i64> = (0..1000).map(|_| rng.next_u64() as i64).collect();
    let s3 = bench_int_scenario(
        "adv_huge",
        "Int array — HUGE MAGNITUDE (fixed/memcpy beats varint)",
        "adversarial",
        huge,
        iters,
    );
    vec![s1, s2, s3]
}

fn bench_strings(n: usize, iters: u32) -> CodecScenario {
    // Random ascii strings of varied length (8..24) — not a fixed `string-value-{i}`
    // template that dictionary/prefix tricks could exploit.
    let mut rng = Rng::new(0xD4_D4_D4);
    let plain: Vec<String> = (0..n).map(|_| rng.ascii_string(8, 24)).collect();
    let rv = rv_list(plain.iter().map(|s| RuntimeValue::Text(Rc::new(s.clone()))).collect());
    let mut rows = vec![
        bench_ours(&rv, iters),
        bench_ours_best(&rv, iters),
        bench_serde("bincode", &plain, iters, |v| bincode::serialize(v).unwrap(), |b| bincode::deserialize(b).unwrap()),
        bench_serde("postcard", &plain, iters, |v| postcard::to_allocvec(v).unwrap(), |b| postcard::from_bytes(b).unwrap()),
        bench_serde("messagepack", &plain, iters, |v| rmp_serde::to_vec(v).unwrap(), |b| rmp_serde::from_slice(b).unwrap()),
        bench_serde("json", &plain, iters, |v| serde_json::to_vec(v).unwrap(), |b| serde_json::from_slice(b).unwrap()),
    ];
    rows.extend(arrow_string_rows(&plain, iters));
    rows.extend(proto_string_rows(&plain, iters));
    rows.extend(capnp_string_rows(&plain, iters));
    scenario("strings", "String list", "fair", n, rows)
}

/// Boolean column (feature flags / presence bitmaps) — a real, common workload. We bit-pack
/// (1 bit/bool); every byte-oriented codec spends a whole byte. Seeded-random flags.
fn bench_bools(n: usize, iters: u32) -> CodecScenario {
    let mut rng = Rng::new(0xB0_01_B0_01);
    let plain: Vec<bool> = (0..n).map(|_| rng.next_u64() & 1 == 0).collect();
    let rv = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(
        plain.iter().map(|&b| RuntimeValue::Bool(b)).collect(),
    ))));
    let rows = vec![
        bench_ours(&rv, iters),
        bench_ours_best(&rv, iters),
        bench_serde("bincode", &plain, iters, |v| bincode::serialize(v).unwrap(), |b| bincode::deserialize(b).unwrap()),
        bench_serde("postcard", &plain, iters, |v| postcard::to_allocvec(v).unwrap(), |b| postcard::from_bytes(b).unwrap()),
        bench_serde("messagepack", &plain, iters, |v| rmp_serde::to_vec(v).unwrap(), |b| rmp_serde::from_slice(b).unwrap()),
        bench_serde(
            "cbor",
            &plain,
            iters,
            |v| {
                let mut o = Vec::new();
                ciborium::into_writer(v, &mut o).unwrap();
                o
            },
            |b| ciborium::from_reader(b).unwrap(),
        ),
        bench_serde("json", &plain, iters, |v| serde_json::to_vec(v).unwrap(), |b| serde_json::from_slice(b).unwrap()),
    ];
    // A showcase (size axis): bit-packing is an 8× win on every bool column, but decode is
    // write-bound (both sides fill an n-element Vec<bool>), so it's a wash — shown size-only.
    scenario("bools", "Boolean column (bit-packed — 1 bit/bool vs a byte each)", "structural", n, rows)
}

/// Float TIME-SERIES (a sensor random walk — slowly varying telemetry). The `BEST` row's Gorilla
/// XOR-delta crushes the near-constant high bits where memcpy/varint can't; the `fixed` dial is
/// the memcpy speed end. A distinct workload from the random-f64 `floats` scenario.
fn bench_timeseries_floats(n: usize, iters: u32) -> CodecScenario {
    let mut rng = Rng::new(0xF1_01_F1_01);
    let mut x = 100.0f64;
    let plain: Vec<f64> = (0..n)
        .map(|_| {
            x += ((rng.next_u64() % 200) as f64 - 100.0) / 1000.0; // ±0.1 steps
            (x * 1000.0).round() / 1000.0
        })
        .collect();
    let rv = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(plain.clone()))));
    let mut rows = vec![
        bench_ours(&rv, iters),
        bench_ours_best(&rv, iters),
        bench_ours_mode("logos (fixed)", &rv, iters, WireNumerics::Fixed),
        bench_ours_zerocopy("logos (zero-copy)", || encode_zerocopy(&rv), |b| { black_box(zerocopy_sum_f64(b)); }, iters),
        bench_serde("bincode", &plain, iters, |v| bincode::serialize(v).unwrap(), |b| bincode::deserialize(b).unwrap()),
        bench_serde("postcard", &plain, iters, |v| postcard::to_allocvec(v).unwrap(), |b| postcard::from_bytes(b).unwrap()),
        bench_serde("messagepack", &plain, iters, |v| rmp_serde::to_vec(v).unwrap(), |b| rmp_serde::from_slice(b).unwrap()),
        bench_serde("json", &plain, iters, |v| serde_json::to_vec(v).unwrap(), |b| serde_json::from_slice(b).unwrap()),
    ];
    rows.extend(arrow_float_rows(&plain, iters));
    scenario("timeseries", "Float time-series (sensor random walk — Gorilla XOR)", "fair", n, rows)
}

/// One random-access row: encode once, then measure encode, full "decode-to-usable", and the
/// headline `read_one_ns` (open the received message + read ONE field at a rotating index).
fn ra_row(
    codec: &'static str,
    iters: u32,
    idxs: &[usize],
    enc: impl Fn() -> Vec<u8>,
    dec_all: impl Fn(&[u8]),
    read_one: impl Fn(&[u8], usize),
) -> RowOut {
    let bytes = enc();
    let size = bytes.len();
    let enc_ns = time_ns(iters, || {
        black_box(enc());
    });
    let dec_ns = time_ns(iters, || {
        dec_all(black_box(&bytes));
    });
    let read_iters = iters.min(2000).max(1);
    let mut k = 0usize;
    let read_one_ns = batch_ns(10, read_iters, || {
        let i = idxs[k % idxs.len()];
        k += 1;
        read_one(black_box(&bytes), i);
    });
    RowOut { codec: codec.to_string(), size, enc_ns, dec_ns, read_one_ns: Some(read_one_ns), chosen: None, fair_size: None }
}

/// Random access — Cap'n Proto's whole pitch: receive a message and read ONE field of one
/// row at a random index. LOGOS opens its record-list view (`T_STRUCTS_VIEW`: row + field
/// offset tables) in O(1); the self-describing serde codecs must decode the whole message to
/// index it (their honest cost); capnp/arrow read straight from the flat buffer (their best).
fn bench_random_access(n: usize, iters: u32) -> CodecScenario {
    let mut rng = Rng::new(0x9A_9A_9A);
    let names = ["alice", "bob", "carol", "dave", "erin"];
    let plain: Vec<Record> = (0..n)
        .map(|i| Record { id: rng.realistic_uint(), name: names[i % names.len()].to_string(), active: i % 2 == 0 })
        .collect();
    let rv = rv_list(plain.iter().map(|r| record_struct(r.id, &r.name, r.active)).collect());
    let idxs: Vec<usize> = (0..256).map(|k: usize| k.wrapping_mul(2_654_435_761) % n).collect();
    // Cap the bulk encode/decode timing so the (heavy, full-decode) serde rows stay quick;
    // size is exact regardless and read_one is separately capped inside ra_row.
    let it = iters.min(4000);

    let mut rows = vec![ra_row(
        "logos (struct-view)",
        it,
        &idxs,
        || with_struct_view(true, || message_to_wire_with("", &rv, WireCodec::Native, WireIntegrity::Raw).unwrap()),
        |b| {
            black_box(message_from_wire(b).unwrap());
        },
        |b, i| {
            let v = view_message(b).unwrap();
            black_box(v.structs_row_field(i, "id").unwrap().as_int().unwrap());
        },
    )];

    // The fixed-stride view (`indexed fast`): no offset tables, arithmetic O(1) — smaller wire
    // and a faster open than the variable view, read through the unified arithmetic reader.
    rows.push(ra_row(
        "logos (struct-view fixed)",
        it,
        &idxs,
        || {
            with_struct_view(true, || {
                with_numerics(WireNumerics::Fixed, || {
                    message_to_wire_with("", &rv, WireCodec::Native, WireIntegrity::Raw).unwrap()
                })
            })
        },
        |b| {
            black_box(message_from_wire(b).unwrap());
        },
        |b, i| {
            let v = view_message(b).unwrap();
            black_box(v.structs_row_field_value(i, "id").unwrap());
        },
    ));

    let bincode_plain = plain.clone();
    rows.push(ra_row(
        "bincode",
        it,
        &idxs,
        || bincode::serialize(&bincode_plain).unwrap(),
        |b| {
            let v: Vec<Record> = bincode::deserialize(b).unwrap();
            black_box(v.len());
        },
        |b, i| {
            let v: Vec<Record> = bincode::deserialize(b).unwrap();
            black_box(v[i].id);
        },
    ));

    let postcard_plain = plain.clone();
    rows.push(ra_row(
        "postcard",
        it,
        &idxs,
        || postcard::to_allocvec(&postcard_plain).unwrap(),
        |b| {
            let v: Vec<Record> = postcard::from_bytes(b).unwrap();
            black_box(v.len());
        },
        |b, i| {
            let v: Vec<Record> = postcard::from_bytes(b).unwrap();
            black_box(v[i].id);
        },
    ));

    let mp_plain = plain.clone();
    rows.push(ra_row(
        "messagepack",
        it,
        &idxs,
        || rmp_serde::to_vec(&mp_plain).unwrap(),
        |b| {
            let v: Vec<Record> = rmp_serde::from_slice(b).unwrap();
            black_box(v.len());
        },
        |b, i| {
            let v: Vec<Record> = rmp_serde::from_slice(b).unwrap();
            black_box(v[i].id);
        },
    ));

    let cbor_plain = plain.clone();
    rows.push(ra_row(
        "cbor",
        it,
        &idxs,
        || {
            let mut o = Vec::new();
            ciborium::into_writer(&cbor_plain, &mut o).unwrap();
            o
        },
        |b| {
            let v: Vec<Record> = ciborium::from_reader(b).unwrap();
            black_box(v.len());
        },
        |b, i| {
            let v: Vec<Record> = ciborium::from_reader(b).unwrap();
            black_box(v[i].id);
        },
    ));

    let json_plain = plain.clone();
    rows.push(ra_row(
        "json",
        it,
        &idxs,
        || serde_json::to_vec(&json_plain).unwrap(),
        |b| {
            let v: Vec<Record> = serde_json::from_slice(b).unwrap();
            black_box(v.len());
        },
        |b, i| {
            let v: Vec<Record> = serde_json::from_slice(b).unwrap();
            black_box(v[i].id);
        },
    ));

    rows.extend(arrow_ra_rows(&plain, &idxs, it));
    rows.extend(proto_ra_rows(&plain, &idxs, it));
    rows.extend(capnp_ra_rows(&plain, &idxs, it));

    CodecScenario {
        id: "random_access".to_string(),
        title: "Open + read one field — random access (Cap'n Proto's home turf)".to_string(),
        n,
        kind: "random_access".to_string(),
        rows,
    }
}

/// Honest showcase of the affine math-hack: a STRUCTURED column (an arithmetic
/// progression — sequential ids, fixed-step timestamps, row indices) ships as just
/// `(base, stride, n)`. Clearly separated from the fair random benchmark above so the
/// hack is never conflated with the general-data result.
fn bench_affine_showcase(n: usize, iters: u32) -> CodecScenario {
    let plain: Vec<i64> = (0..n as i64).map(|i| 1_000_000 + i * 7).collect(); // base + stride*i
    let rv = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(plain.clone()))));
    let rows = vec![
        bench_ours(&rv, iters),
        bench_ours_best(&rv, iters),
        bench_ours_affine(&rv, iters),
        bench_serde("postcard", &plain, iters, |v| postcard::to_allocvec(v).unwrap(), |b| postcard::from_bytes(b).unwrap()),
        bench_serde("messagepack", &plain, iters, |v| rmp_serde::to_vec(v).unwrap(), |b| rmp_serde::from_slice(b).unwrap()),
        bench_serde("json", &plain, iters, |v| serde_json::to_vec(v).unwrap(), |b| serde_json::from_slice(b).unwrap()),
    ];
    scenario(
        "affine_showcase",
        "Structured/affine column (sequential ids — best case for the math-hack)",
        "showcase",
        n,
        rows,
    )
}

/// Showcase of the polynomial generator: a column that is exactly a degree-2 polynomial
/// (`3i² − 5i + 7`) ships as its finite-difference SEEDS — a handful of numbers regardless of n
/// — instead of the data. Kept separate from the fair results, like the affine showcase.
fn bench_poly_showcase(n: usize, iters: u32) -> CodecScenario {
    let plain: Vec<i64> = (0..n as i64).map(|i| 3 * i * i - 5 * i + 7).collect();
    let rv = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(plain.clone()))));
    let rows = vec![
        bench_ours(&rv, iters),
        bench_ours_best(&rv, iters),
        bench_serde("postcard", &plain, iters, |v| postcard::to_allocvec(v).unwrap(), |b| postcard::from_bytes(b).unwrap()),
        bench_serde("messagepack", &plain, iters, |v| rmp_serde::to_vec(v).unwrap(), |b| rmp_serde::from_slice(b).unwrap()),
        bench_serde("json", &plain, iters, |v| serde_json::to_vec(v).unwrap(), |b| serde_json::from_slice(b).unwrap()),
    ];
    scenario(
        "poly_showcase",
        "Structured/polynomial column (3i\u{00b2}\u{2212}5i+7 — ships the generator, not the data)",
        "showcase",
        n,
        rows,
    )
}

/// Showcase of the string dictionary: a low-cardinality categorical column (HTTP methods / event
/// types — a few distinct labels repeated) ships the distinct labels ONCE plus a bit-packed index
/// per row, instead of every repeat. The `BEST` row finds it via the `Auto` per-column menu.
fn bench_categorical_strings(n: usize, iters: u32) -> CodecScenario {
    let mut rng = Rng::new(0xCA7E_60E1);
    let cats = ["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"];
    let plain: Vec<String> = (0..n).map(|_| cats[(rng.next_u64() as usize) % cats.len()].to_string()).collect();
    let rv = rv_list(plain.iter().map(|s| RuntimeValue::Text(Rc::new(s.clone()))).collect());
    let rows = vec![
        bench_ours(&rv, iters),
        bench_ours_best(&rv, iters),
        bench_serde("postcard", &plain, iters, |v| postcard::to_allocvec(v).unwrap(), |b| postcard::from_bytes(b).unwrap()),
        bench_serde("messagepack", &plain, iters, |v| rmp_serde::to_vec(v).unwrap(), |b| rmp_serde::from_slice(b).unwrap()),
        bench_serde("json", &plain, iters, |v| serde_json::to_vec(v).unwrap(), |b| serde_json::from_slice(b).unwrap()),
    ];
    scenario("categorical", "Categorical strings (low-cardinality labels — dictionary)", "structural", n, rows)
}

/// Showcase of the int-SET encoding: a Set is order-invariant, so the codec sorts to the
/// canonical monotone column and runs it through the int menu — a consecutive set {0..n}
/// collapses to base+stride+count (~no data). serde stores every member. (Different data
/// structure than the affine LIST showcase: this is a `Set`.)
fn bench_int_set(n: usize, iters: u32) -> CodecScenario {
    let rv = RuntimeValue::Set(Rc::new(RefCell::new((0..n as i64).map(RuntimeValue::Int).collect())));
    let set: std::collections::BTreeSet<i64> = (0..n as i64).collect();
    let rows = vec![
        bench_ours(&rv, iters),
        bench_ours_best(&rv, iters),
        bench_serde("postcard", &set, iters, |v| postcard::to_allocvec(v).unwrap(), |b| postcard::from_bytes(b).unwrap()),
        bench_serde("messagepack", &set, iters, |v| rmp_serde::to_vec(v).unwrap(), |b| rmp_serde::from_slice(b).unwrap()),
        bench_serde("json", &set, iters, |v| serde_json::to_vec(v).unwrap(), |b| serde_json::from_slice(b).unwrap()),
    ];
    scenario("int_set", "Integer SET {0..n} (consecutive — ships base+stride+count)", "showcase", n, rows)
}

/// Showcase of the int-keyed MAP encoding: an int→int map ships as TWO columns (keys + values),
/// each run through the int menu. An affine map {i ↦ 2i} collapses BOTH columns to
/// base+stride+count (~no data). serde stores every pair.
fn bench_int_map(n: usize, iters: u32) -> CodecScenario {
    let mut m = MapStorage::default();
    for i in 0..n as i64 {
        m.insert(RuntimeValue::Int(i), RuntimeValue::Int(2 * i));
    }
    let rv = RuntimeValue::Map(Rc::new(RefCell::new(m)));
    let map: std::collections::BTreeMap<i64, i64> = (0..n as i64).map(|i| (i, 2 * i)).collect();
    let rows = vec![
        bench_ours(&rv, iters),
        bench_ours_best(&rv, iters),
        bench_serde("postcard", &map, iters, |v| postcard::to_allocvec(v).unwrap(), |b| postcard::from_bytes(b).unwrap()),
        bench_serde("messagepack", &map, iters, |v| rmp_serde::to_vec(v).unwrap(), |b| rmp_serde::from_slice(b).unwrap()),
        bench_serde("json", &map, iters, |v| serde_json::to_vec(v).unwrap(), |b| serde_json::from_slice(b).unwrap()),
    ];
    scenario("int_map", "Integer MAP {i\u{21a6}2i} (affine — ships both columns as base+stride+count)", "showcase", n, rows)
}

/// Resolved competitor crate versions, captured from the workspace `Cargo.lock` by build.rs.
fn competitor_versions() -> HashMap<String, String> {
    let mut m = HashMap::new();
    let mut add = |k: &str, v: Option<&str>| {
        if let Some(v) = v {
            m.insert(k.to_string(), v.to_string());
        }
    };
    add("bincode", option_env!("WIREBENCH_VER_bincode"));
    add("postcard", option_env!("WIREBENCH_VER_postcard"));
    add("messagepack", option_env!("WIREBENCH_VER_rmp_serde"));
    add("cbor", option_env!("WIREBENCH_VER_ciborium"));
    add("json", option_env!("WIREBENCH_VER_serde_json"));
    if cfg!(feature = "arrow-bench") {
        add("arrow (ipc)", option_env!("WIREBENCH_VER_arrow"));
    }
    if cfg!(feature = "protobuf") {
        add("protobuf/grpc", option_env!("WIREBENCH_VER_prost"));
    }
    if cfg!(feature = "capnproto") {
        add("capnproto", option_env!("WIREBENCH_VER_capnp"));
    }
    m
}

fn active_features() -> Vec<String> {
    let mut v = Vec::new();
    if cfg!(feature = "arrow-bench") {
        v.push("arrow-bench".to_string());
    }
    if cfg!(feature = "protobuf") {
        v.push("protobuf".to_string());
    }
    if cfg!(feature = "capnproto") {
        v.push("capnproto".to_string());
    }
    v
}

/// A dependency-free `SystemTime` → `YYYY-MM-DDTHH:MM:SSZ` (Howard Hinnant's
/// `civil_from_days`). Used only as the local-run fallback — under CI the run.sh env
/// override wins, so the date matches `latest.json` exactly.
fn iso_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

fn collect_metadata() -> CodecMetadata {
    let env = |k: &str| std::env::var(k).ok().filter(|s| !s.is_empty());
    CodecMetadata {
        date: env("WIREBENCH_DATE").unwrap_or_else(iso_now),
        commit: env("WIREBENCH_COMMIT").unwrap_or_else(|| "unknown".to_string()),
        logos_version: env("LOGOS_VERSION").unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string()),
        cpu: env("WIREBENCH_CPU").unwrap_or_else(|| "unknown".to_string()),
        os: env("WIREBENCH_OS").unwrap_or_else(|| "unknown".to_string()),
        versions: competitor_versions(),
        features: active_features(),
    }
}

/// Run the whole head-to-head and return it as a structured report — the single source of
/// truth for both the stdout table and the JSON the page bakes in.
pub fn build_report(iters: u32) -> CodecReport {
    let mut scenarios = vec![
        bench_ints(1000, iters),
        bench_floats(1000, iters),
        bench_timeseries_floats(1000, iters),
        bench_points(1000, iters),
        bench_records(200, iters),
        bench_strings(200, iters),
        bench_bools(1000, iters),
    ];
    scenarios.extend(bench_adversarial(iters));
    scenarios.push(bench_random_access(2000, iters));
    scenarios.push(bench_affine_showcase(1000, iters));
    scenarios.push(bench_poly_showcase(1000, iters));
    scenarios.push(bench_categorical_strings(1000, iters));
    scenarios.push(bench_int_set(1000, iters));
    scenarios.push(bench_int_map(1000, iters));
    // v2 adds the `chosen` config on the all-knobs winner (which dials + per-column encodings won).
    CodecReport { schema_version: 2, metadata: collect_metadata(), iters, scenarios }
}

/// The binary entry point: print the human table, and (when `WIREBENCH_JSON` is set) write
/// the same results as `latest-codec.json` for the web Benchmarks page.
pub fn run() {
    println!("# Logos wire codec — fair head-to-head (same logical data, same machine)");
    println!("# size = encoded bytes (no envelope); enc/dec = ns per whole-message op.");
    println!("# Payloads are SEEDED-RANDOM (a fair, reproducible comparison); the final");
    println!("# section showcases the affine hack on structured data, kept separate.");
    let iters = std::env::var("WIREBENCH_ITERS").ok().and_then(|s| s.parse().ok()).unwrap_or(20_000u32);
    let report = build_report(iters);
    for s in &report.scenarios {
        print_table(s);
    }
    println!("\n# (protobuf / Cap'n Proto / Arrow run under --features heavy; see the script.)");
    if let Ok(path) = std::env::var("WIREBENCH_JSON") {
        let json = serde_json::to_string_pretty(&report).expect("serialize codec report");
        std::fs::write(&path, json).unwrap_or_else(|e| panic!("write {path}: {e}"));
        eprintln!("# wrote {} scenarios to {path}", report.scenarios.len());
    }
}

#[cfg(test)]
mod zerocopy_tests {
    use super::*;

    #[test]
    fn zerocopy_reads_are_in_place_and_correct() {
        // The zero-copy row is only honest if it actually reads IN PLACE (the aligned-slice fast
        // path), not the decode fallback. Assert the fast path fires and the borrowed slice is
        // bit-exact with the source — this is what makes the fair capnp/arrow comparison real.
        let ints: Vec<i64> = (0..1000).map(|i| i * 7 - 3).collect();
        let rv = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(ints.clone()))));
        let bytes = encode_zerocopy(&rv);
        let view = view_message(&bytes).expect("native view opens");
        let slice = view.as_i64_slice().expect("int zero-copy MUST read the aligned column IN PLACE");
        assert_eq!(slice, ints.as_slice(), "the borrowed i64 slice equals the source (no copy, no loss)");
        let want = ints.iter().fold(0i64, |a, &x| a.wrapping_add(x));
        assert_eq!(zerocopy_sum_i64(&bytes), want, "zero-copy int sum == materialized sum");

        let floats: Vec<f64> = (0..1000).map(|i| i as f64 * 0.5 - 1.0).collect();
        let rvf = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(floats.clone()))));
        let fb = encode_zerocopy(&rvf);
        let fview = view_message(&fb).expect("native view opens");
        let fslice = fview.as_f64_slice().expect("float zero-copy MUST read the aligned column IN PLACE");
        assert_eq!(fslice, floats.as_slice(), "the borrowed f64 slice equals the source");
        let fwant: f64 = floats.iter().sum();
        assert_eq!(zerocopy_sum_f64(&fb), fwant, "zero-copy float sum == materialized sum");
    }

    #[test]
    fn zerocopy_points_read_matches_materialized() {
        // The columnar points zero-copy path reads each field's CONTIGUOUS blob in place; it must
        // (a) actually find both fixed columns (the fast path fires) and (b) sum EXACTLY what a full
        // decode would. Fields are canonically sorted → column 0 = x, column 1 = y.
        let pts: Vec<(i64, i64)> = (0..500).map(|i| (i * 3 - 7, -i * 5 + 2)).collect();
        let rv = rv_list(pts.iter().map(|&(x, y)| point_struct(x, y)).collect());
        let bytes = encode_columnar(&rv);
        let view = view_message(&bytes).expect("native view opens");
        let xcol = view.structs_fixed_i64_col(0).expect("x column MUST read as a contiguous blob");
        let ycol = view.structs_fixed_i64_col(1).expect("y column MUST read as a contiguous blob");
        assert_eq!(xcol.len(), pts.len() * 8, "x column is n contiguous i64s");
        assert_eq!(ycol.len(), pts.len() * 8, "y column is n contiguous i64s");
        let want = pts.iter().fold(0i64, |a, &(x, y)| a.wrapping_add(x).wrapping_add(y));
        assert_eq!(zerocopy_sum_points(&bytes), want, "zero-copy columnar points sum == materialized sum");
    }

    #[test]
    fn fair_scenarios_carry_a_zero_copy_row() {
        let report = build_report(20);
        for id in ["ints", "floats", "points"] {
            let s = report.scenarios.iter().find(|s| s.id == id).unwrap_or_else(|| panic!("no '{id}' scenario"));
            assert!(
                s.rows.iter().any(|r| r.codec == "logos (zero-copy)"),
                "'{id}' must carry a 'logos (zero-copy)' row (the fair zero-copy read path)"
            );
        }
    }
}
