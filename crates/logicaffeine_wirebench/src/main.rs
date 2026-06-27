//! Fair head-to-head: the Logos wire codec vs industry serializers, on the same
//! logical data (the Logos value space — int/float arrays, struct lists, records,
//! strings). Measures encoded size, encode ns/op, decode ns/op, and a single-field
//! read. Pure-Rust competitors run by default; protobuf/Cap'n Proto/Arrow build
//! under `--features heavy` (they need a toolchain — see bench-wire-vs-protocols.sh).

use std::cell::RefCell;
use std::collections::HashMap;
use std::hint::black_box;
use std::rc::Rc;
use std::time::Instant;

use logicaffeine_compile::concurrency::marshal::{
    message_from_wire, message_to_wire_with, with_compression_codec, with_floats, with_numerics,
    with_structure, WireCodec, WireCompression, WireFloats, WireIntegrity, WireNumerics, WireStructure,
};
use logicaffeine_compile::interpreter::{ListRepr, RuntimeValue, StructValue};
use serde::{Deserialize, Serialize};

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

/// Average ns per call over `iters`, with a warm-up.
fn time_ns(iters: u32, mut f: impl FnMut()) -> f64 {
    for _ in 0..(iters / 10).max(1) {
        f();
    }
    let t = Instant::now();
    for _ in 0..iters {
        f();
    }
    t.elapsed().as_nanos() as f64 / iters as f64
}

struct Row {
    codec: &'static str,
    size: usize,
    enc_ns: f64,
    dec_ns: f64,
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
    Row { codec, size, enc_ns, dec_ns }
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
    Row { codec: label, size, enc_ns, dec_ns }
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
    Row { codec: "logos (affine)", size, enc_ns, dec_ns }
}

/// "Turn EVERY knob": encode under every dial combination (numerics × structure × floats,
/// each also Zstd-compressed), verify each round-trips, and report the SMALLEST. This is
/// the honest answer to "if we use everything we've got, who still beats us, and where?"
fn bench_ours_best(value: &RuntimeValue, iters: u32) -> Row {
    let encode = |num: WireNumerics, st: WireStructure, fl: WireFloats, comp: bool| -> Vec<u8> {
        let body = || {
            with_numerics(num, || {
                with_structure(st, || {
                    with_floats(fl, || {
                        message_to_wire_with("", value, WireCodec::Native, WireIntegrity::Raw).unwrap()
                    })
                })
            })
        };
        if comp {
            with_compression_codec(WireCompression::Zstd, body)
        } else {
            body()
        }
    };
    let mut cfgs = Vec::new();
    for num in [WireNumerics::Varint, WireNumerics::Fixed, WireNumerics::GroupVarint] {
        for st in [WireStructure::Off, WireStructure::Affine] {
            for fl in [WireFloats::Memcpy, WireFloats::XorDelta] {
                for comp in [false, true] {
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
    Row { codec: "logos (BEST: all knobs)", size: bytes.len(), enc_ns, dec_ns }
}

/// A codec given purely as opaque-blob encode/decode closures (for non-serde
/// competitors like Arrow). `dec` must *touch every value* it reads — the honest
/// measure of "decode to something usable", so a format that decodes lazily is
/// credited for exactly the work it actually does.
fn bench_blob(codec: &'static str, iters: u32, enc: impl Fn() -> Vec<u8>, dec: impl Fn(&[u8])) -> Row {
    let bytes = enc();
    let size = bytes.len();
    let enc_ns = time_ns(iters, || {
        black_box(enc());
    });
    let dec_ns = time_ns(iters, || {
        dec(black_box(&bytes));
    });
    Row { codec, size, enc_ns, dec_ns }
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

fn print_table(name: &str, n: usize, rows: &[Row]) {
    let json = rows.iter().find(|r| r.codec == "json").map_or(1, |r| r.size.max(1));
    println!("\n=== {name} (n={n}) ===");
    println!("  {:<16} {:>10} {:>12} {:>12} {:>10}", "codec", "size B", "enc ns/op", "dec ns/op", "× json");
    for r in rows {
        println!(
            "  {:<16} {:>10} {:>12.0} {:>12.0} {:>9.1}×",
            r.codec,
            r.size,
            r.enc_ns,
            r.dec_ns,
            json as f64 / r.size as f64
        );
    }
}

fn bench_points(n: usize, iters: u32) {
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
    print_table("Point list {x:i64,y:i64}", n, &rows);
}

fn bench_records(n: usize, iters: u32) {
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
    print_table("Record list {id,name,active}", n, &rows);
}

/// Run one int-array scenario (the SAME logical data through every codec, all our knobs
/// turned) under a descriptive title — reused for the fair random case and the
/// adversarial "codecs choke here" cases.
fn bench_int_scenario(title: &str, plain: Vec<i64>, iters: u32) {
    let n = plain.len();
    let rv = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(plain.clone()))));
    let mut rows = vec![
        bench_ours(&rv, iters),
        bench_ours_best(&rv, iters),
        bench_ours_mode("logos (fixed)", &rv, iters, WireNumerics::Fixed),
        bench_ours_mode("logos (gv/simd)", &rv, iters, WireNumerics::GroupVarint),
        bench_ours_affine(&rv, iters),
        bench_serde("bincode", &plain, iters, |v| bincode::serialize(v).unwrap(), |b| bincode::deserialize(b).unwrap()),
        bench_serde("postcard", &plain, iters, |v| postcard::to_allocvec(v).unwrap(), |b| postcard::from_bytes(b).unwrap()),
        bench_serde("messagepack", &plain, iters, |v| rmp_serde::to_vec(v).unwrap(), |b| rmp_serde::from_slice(b).unwrap()),
        bench_serde("json", &plain, iters, |v| serde_json::to_vec(v).unwrap(), |b| serde_json::from_slice(b).unwrap()),
    ];
    rows.extend(arrow_int_rows(&plain, iters));
    rows.extend(proto_int_rows(&plain, iters));
    rows.extend(capnp_int_rows(&plain, iters));
    print_table(title, n, &rows);
}

fn bench_ints(n: usize, iters: u32) {
    // Random non-negative ints with a realistic magnitude spread (ids/counts/sizes) —
    // NOT a `0..n` sequence (which the affine hack would elide to 7 bytes for free).
    let mut rng = Rng::new(0xA1_A1_A1);
    let plain: Vec<i64> = (0..n).map(|_| rng.realistic_uint()).collect();
    bench_int_scenario("Int array (random)", plain, iters);
}

/// Float race: random sensor/mesh-like f64. Our float column is memcpy by default, with a
/// Gorilla XOR-delta knob the `BEST` row tries. Competitors: serde codecs + arrow Float64.
fn bench_floats(n: usize, iters: u32) {
    let mut rng = Rng::new(0xF0_F0_F0);
    let plain: Vec<f64> = (0..n).map(|_| rng.realistic_float()).collect();
    let rv = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(plain.clone()))));
    let mut rows = vec![
        bench_ours(&rv, iters),
        bench_ours_best(&rv, iters),
        bench_ours_mode("logos (fixed)", &rv, iters, WireNumerics::Fixed),
        bench_serde("bincode", &plain, iters, |v| bincode::serialize(v).unwrap(), |b| bincode::deserialize(b).unwrap()),
        bench_serde("postcard", &plain, iters, |v| postcard::to_allocvec(v).unwrap(), |b| postcard::from_bytes(b).unwrap()),
        bench_serde("messagepack", &plain, iters, |v| rmp_serde::to_vec(v).unwrap(), |b| rmp_serde::from_slice(b).unwrap()),
        bench_serde("json", &plain, iters, |v| serde_json::to_vec(v).unwrap(), |b| serde_json::from_slice(b).unwrap()),
    ];
    rows.extend(arrow_float_rows(&plain, iters));
    print_table("Float array (random f64)", n, &rows);
}

/// Adversarial scenarios — the data shapes other codecs choke on, with all our knobs on.
fn bench_adversarial(iters: u32) {
    let mut rng = Rng::new(0xE5_E5_E5);
    // 1. ALL NEGATIVE: protobuf `int64` spends 10 bytes per negative; our adaptive
    //    zig-zag spends 1–3. The scenario protobuf is worst at.
    let neg: Vec<i64> = (0..1000).map(|_| -(rng.realistic_uint().max(1))).collect();
    bench_int_scenario("Int array — ALL NEGATIVE (protobuf int64 = 10B/elem)", neg, iters);
    // 2. REPETITIVE: a tiny value set cycled — compression/dictionary territory; the
    //    BEST (all-knobs) row turns on Zstd, the fixed-width codecs cannot shrink.
    let rep: Vec<i64> = (0..2000).map(|i| ((i % 8) as i64) * 1_000_000).collect();
    bench_int_scenario("Int array — REPETITIVE (compression shines; fixed codecs can't)", rep, iters);
    // 3. HUGE MAGNITUDE: full 64-bit random — varint loses to fixed; the dial lets us
    //    pick memcpy (8B, fast) where varint-only codecs (protobuf) stay at ~9–10B.
    let huge: Vec<i64> = (0..1000).map(|_| rng.next_u64() as i64).collect();
    bench_int_scenario("Int array — HUGE MAGNITUDE (fixed/memcpy beats varint)", huge, iters);
}

fn bench_strings(n: usize, iters: u32) {
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
    print_table("String list", n, &rows);
}

/// Honest showcase of the affine math-hack: a STRUCTURED column (an arithmetic
/// progression — sequential ids, fixed-step timestamps, row indices) ships as just
/// `(base, stride, n)`. Clearly separated from the fair random benchmark above so the
/// hack is never conflated with the general-data result.
fn bench_affine_showcase(n: usize, iters: u32) {
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
    print_table("Structured/affine column (sequential ids — best case for the math-hack)", n, &rows);
}

fn main() {
    println!("# Logos wire codec — fair head-to-head (same logical data, same machine)");
    println!("# size = encoded bytes (no envelope); enc/dec = ns per whole-message op.");
    println!("# Payloads are SEEDED-RANDOM (a fair, reproducible comparison); the final");
    println!("# section showcases the affine hack on structured data, kept separate.");
    let iters = std::env::var("WIREBENCH_ITERS").ok().and_then(|s| s.parse().ok()).unwrap_or(20_000u32);
    bench_ints(1000, iters);
    bench_floats(1000, iters);
    bench_points(1000, iters);
    bench_records(200, iters);
    bench_strings(200, iters);
    bench_adversarial(iters);
    bench_affine_showcase(1000, iters);
    println!("\n# (protobuf / Cap'n Proto / Arrow run under --features heavy; see the script.)");
}
