//! Superiority lock-ins — regression tests that FAIL the day we stop beating a rival at
//! its own game. These turn the benchmark's claims into permanent guarantees.
//!
//! SIZE assertions are deterministic (byte counts never vary). SPEED assertions use a
//! same-process min-of-N ratio: both codecs are timed under the same machine load in the
//! same run, so the RATIO is load-invariant (a busy box slows both equally) — and we take
//! the min of many iterations to filter scheduler noise, with a generous margin over the
//! real gap. The competitor crates are wirebench's own dependencies.

use std::cell::RefCell;
use std::hint::black_box;
use std::rc::Rc;
use std::time::Instant;

use logicaffeine_compile::concurrency::marshal::{
    message_from_wire, message_to_wire_with, with_compression_codec, with_floats, with_numerics,
    with_structure, WireCodec, WireCompression, WireFloats, WireIntegrity, WireNumerics, WireStructure,
};
use logicaffeine_compile::interpreter::{ListRepr, RuntimeValue};

// ---- deterministic data (so the lock-ins are reproducible) ----

struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed)
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn uint(&mut self) -> i64 {
        let bits = (self.next() % 40) as u32 + 1;
        (self.next() & ((1u64 << bits) - 1)) as i64
    }
}

fn rv_ints(v: &[i64]) -> RuntimeValue {
    RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(v.to_vec()))))
}
fn rv_floats(v: &[f64]) -> RuntimeValue {
    RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(v.to_vec()))))
}
fn rv_from(values: Vec<RuntimeValue>) -> RuntimeValue {
    RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(values))))
}
fn rv_bools(v: &[bool]) -> RuntimeValue {
    rv_from(v.iter().map(|&b| RuntimeValue::Bool(b)).collect())
}
fn rv_strings(v: &[&str]) -> RuntimeValue {
    rv_from(v.iter().map(|s| RuntimeValue::Text(Rc::new(s.to_string()))).collect())
}

/// Realistic workloads people actually transmit — generated deterministically. Returns
/// (label, our-best-bytes, postcard-bytes) for each, so the lock-in covers breadth, not
/// just synthetic shapes.
fn realistic_workloads() -> Vec<(&'static str, usize, usize)> {
    let mut rng = Rng::new(0xDA7A);
    let mut out = Vec::new();
    // 1. Monotonic timestamps (unix millis, small random steps) — IoT / event streams.
    let mut t = 1_700_000_000_000i64;
    let timestamps: Vec<i64> = (0..1000).map(|_| { t += (rng.next() % 1000) as i64; t }).collect();
    out.push(("timestamps", logos_best(&rv_ints(&timestamps)), postcard::to_allocvec(&timestamps).unwrap().len()));
    // 2. HTTP status codes (200/301/404/500 — low cardinality) — access logs.
    let codes_set = [200i64, 200, 200, 301, 404, 500, 204, 200];
    let statuses: Vec<i64> = (0..1000).map(|_| codes_set[(rng.next() % 8) as usize]).collect();
    out.push(("http status", logos_best(&rv_ints(&statuses)), postcard::to_allocvec(&statuses).unwrap().len()));
    // 3. Latencies in ms (small positive, clustered) — APM / tracing.
    let latencies: Vec<i64> = (0..1000).map(|_| 1 + (rng.next() % 500) as i64).collect();
    out.push(("latencies", logos_best(&rv_ints(&latencies)), postcard::to_allocvec(&latencies).unwrap().len()));
    // 4. Prices in cents (positive, clustered around a band) — commerce / finance.
    let prices: Vec<i64> = (0..1000).map(|_| 99 + (rng.next() % 90000) as i64).collect();
    out.push(("prices", logos_best(&rv_ints(&prices)), postcard::to_allocvec(&prices).unwrap().len()));
    // 5. Geo latitudes (floats in [-90,90]) — maps / fleet.
    let lats: Vec<f64> = (0..1000).map(|_| (rng.next() % 180_000_000) as f64 / 1_000_000.0 - 90.0).collect();
    out.push(("geo lat", logos_best(&rv_floats(&lats)), postcard::to_allocvec(&lats).unwrap().len()));
    // 6. Slowly-varying sensor readings (random walk) — telemetry.
    let mut s = 20.0f64;
    let sensors: Vec<f64> = (0..1000).map(|_| { s += ((rng.next() % 200) as f64 - 100.0) / 100.0; (s * 100.0).round() / 100.0 }).collect();
    out.push(("sensor walk", logos_best(&rv_floats(&sensors)), postcard::to_allocvec(&sensors).unwrap().len()));
    // 7. Feature flags (booleans) — config / presence bitmaps.
    let flags: Vec<bool> = (0..1000).map(|_| rng.next() & 1 == 0).collect();
    out.push(("flags", logos_best(&rv_bools(&flags)), postcard::to_allocvec(&flags).unwrap().len()));
    // 8. Categorical labels (repeated short strings) — event types / methods.
    let methods = ["GET", "POST", "GET", "GET", "PUT", "DELETE", "GET", "PATCH"];
    let labels: Vec<&str> = (0..1000).map(|_| methods[(rng.next() % 8) as usize]).collect();
    out.push(("methods", logos_best(&rv_strings(&labels)), postcard::to_allocvec(&labels).unwrap().len()));
    // 9. Mixed-magnitude counters (some tiny, some huge) — metrics.
    let counters: Vec<i64> = (0..1000).map(|_| rng.uint()).collect();
    out.push(("counters", logos_best(&rv_ints(&counters)), postcard::to_allocvec(&counters).unwrap().len()));
    out
}

fn enc(rv: &RuntimeValue) -> Vec<u8> {
    message_to_wire_with("", rv, WireCodec::Native, WireIntegrity::Raw).unwrap()
}

/// Encode `rv` under one explicit dial setting.
fn enc_mode(rv: &RuntimeValue, num: WireNumerics, st: WireStructure, fl: WireFloats, zstd: bool) -> Vec<u8> {
    with_numerics(num, || {
        with_structure(st, || {
            with_floats(fl, || {
                if zstd {
                    with_compression_codec(WireCompression::Zstd, || enc(rv))
                } else {
                    enc(rv)
                }
            })
        })
    })
}

/// The smallest message our codec can produce for `rv` across the whole dial — and it
/// must round-trip (an encoding that doesn't decode is not a valid "best").
fn logos_best(rv: &RuntimeValue) -> usize {
    let mut best = usize::MAX;
    for num in [WireNumerics::Varint, WireNumerics::Fixed, WireNumerics::GroupVarint] {
        for st in [WireStructure::Off, WireStructure::Affine] {
            for fl in [WireFloats::Memcpy, WireFloats::XorDelta] {
                for zstd in [false, true] {
                    let bytes = enc_mode(rv, num, st, fl, zstd);
                    if message_from_wire(&bytes).is_some() && bytes.len() < best {
                        best = bytes.len();
                    }
                }
            }
        }
    }
    best
}

/// The fastest batch-average ns/op: time a whole batch of `iters` calls with ONE timer
/// (so the per-call `Instant::now()` overhead — ~30ns, larger than a memcpy encode! — is
/// amortized, not measured), repeat `rounds` times, and take the MIN batch (filters a
/// scheduler hiccup). Accurate for sub-microsecond ops AND load-robust.
fn batch_ns(rounds: u32, iters: u32, mut f: impl FnMut()) -> f64 {
    for _ in 0..iters {
        f(); // warm-up
    }
    let mut best = f64::MAX;
    for _ in 0..rounds {
        let t = Instant::now();
        for _ in 0..iters {
            f();
        }
        best = best.min(t.elapsed().as_nanos() as f64 / iters as f64);
    }
    best
}

// =====================================================================================
// RULE 1 — BEAT postcard at smallest framing on simple data, in every case.
// =====================================================================================
#[test]
fn we_are_never_beaten_by_postcard_on_size() {
    // Across NINE realistic workloads (timestamps, http status, latencies, prices, geo,
    // sensors, flags, categorical labels, mixed counters) plus the synthetic extremes,
    // postcard must never produce a smaller message than our best. Postcard's superpower
    // is minimal framing; our dial always matches or beats it.
    let mut cases = realistic_workloads();
    let mut rng = Rng::new(0x501);
    let random: Vec<i64> = (0..1000).map(|_| rng.uint()).collect();
    let negative: Vec<i64> = (0..1000).map(|_| -(rng.uint().max(1))).collect();
    let sequential: Vec<i64> = (0..1000).map(|i| 1_000_000 + i * 7).collect();
    cases.push(("random ints", logos_best(&rv_ints(&random)), postcard::to_allocvec(&random).unwrap().len()));
    cases.push(("negative ints", logos_best(&rv_ints(&negative)), postcard::to_allocvec(&negative).unwrap().len()));
    cases.push(("sequential ints", logos_best(&rv_ints(&sequential)), postcard::to_allocvec(&sequential).unwrap().len()));

    println!("\n── AXIS: SIZE vs postcard (smaller is better) ──");
    for (label, ours, postcard_len) in cases {
        assert!(
            ours <= postcard_len,
            "postcard must NOT beat our smallest on '{label}': ours {ours} vs postcard {postcard_len}"
        );
        println!(
            "  {label:<16} ours {ours:>6}B  ≤  postcard {postcard_len:>6}B   ({:.2}× smaller)",
            postcard_len as f64 / ours.max(1) as f64
        );
    }
}

// =====================================================================================
// RULE 1b — BEAT every codec on boolean columns: we bit-pack (1 bit/bool); the rest
//           spend a whole byte. Flags/bitmaps are a very common real workload.
// =====================================================================================
#[test]
fn we_bit_pack_booleans_8x_smaller() {
    let mut rng = Rng::new(0xB001);
    let flags: Vec<bool> = (0..1000).map(|_| rng.next() & 1 == 0).collect();
    let ours = logos_best(&rv_bools(&flags));
    let postcard = postcard::to_allocvec(&flags).unwrap().len();
    let bincode = bincode::serialize(&flags).unwrap().len();
    // 1000 bits = 125 bytes + tiny header, vs 1000 bytes for the byte-per-bool codecs.
    assert!(ours < 200, "1000 bools must bit-pack to ~125B: got {ours}");
    assert!(ours * 4 < postcard.min(bincode), "bools must be ≥4× smaller than byte-per-bool codecs");
    println!(
        "\n── AXIS: BOOLEANS (bit-pack) ──\n  ours {ours}B  vs  postcard {postcard}B / bincode {bincode}B   ({:.1}× smaller)",
        postcard.min(bincode) as f64 / ours as f64
    );
}

// =====================================================================================
// RULE 2 — BEAT bincode at fast FIXED encode (the memcpy speed end).
// =====================================================================================
#[test]
fn we_beat_bincode_at_fixed_encode_speed() {
    let mut rng = Rng::new(0xB10);
    let v: Vec<i64> = (0..1000).map(|_| rng.next() as i64).collect();
    let rv = rv_ints(&v);
    // Our fixed-width (memcpy) layout vs bincode's fixed-width serialize, same data,
    // same run. The measured gap is ~2.9×; assert a comfortable 1.5× so load never flips it.
    let ours = batch_ns(50, 4000, || {
        black_box(with_numerics(WireNumerics::Fixed, || enc(&rv)));
    });
    let bincode = batch_ns(50, 4000, || {
        black_box(bincode::serialize(&v).unwrap());
    });
    assert!(
        ours * 1.5 < bincode,
        "fixed encode must beat bincode by >1.5×: ours {ours:.0}ns vs bincode {bincode:.0}ns"
    );
}

// =====================================================================================
// RULE 3 — BEAT protobuf at varint on signed data (its int64 weakness), no toolchain.
//          protobuf `int64` spends 10 bytes per negative; we auto-pick zig-zag (~3).
// =====================================================================================
#[test]
fn we_beat_protobuf_int64_on_signed_data() {
    let mut rng = Rng::new(0x9C0);
    let negative: Vec<i64> = (0..1000).map(|_| -(rng.uint().max(1))).collect();
    let ours = enc(&rv_ints(&negative)).len();
    // protobuf int64 (non-zig-zag) is ≥ 10 bytes for any negative → ≥ 10*n on the column.
    // Lock that we stay FAR under that (the adaptive zig-zag never pays the 10-byte tax).
    let protobuf_floor = negative.len() * 10;
    assert!(
        ours < protobuf_floor / 2,
        "our signed-int wire must be <½ protobuf's 10B/negative floor: ours {ours} vs floor {protobuf_floor}"
    );
    println!(
        "\n── AXIS: SIGNED ints vs protobuf int64 ──\n  ours {ours}B  vs  protobuf floor {protobuf_floor}B   ({:.1}× smaller)",
        protobuf_floor as f64 / ours as f64
    );
}

// =====================================================================================
// RULE 4 — BEAT the naive float wire (and Gorilla's niche) on time-series floats: the
//          XOR-delta layout shrinks slowly-varying floats where memcpy/postcard cannot.
// =====================================================================================
#[test]
fn xor_delta_beats_memcpy_and_postcard_on_time_series_floats() {
    // A slowly-varying signal (a random walk) — the time-series case Gorilla/InfluxDB win.
    let mut rng = Rng::new(0xF10);
    let mut x = 100.0f64;
    let series: Vec<f64> = (0..1000)
        .map(|_| {
            x += ((rng.next() % 200) as f64 - 100.0) / 1000.0; // ±0.1 steps
            (x * 1000.0).round() / 1000.0
        })
        .collect();
    let rv = rv_floats(&series);
    let xor = enc_mode(&rv, WireNumerics::Varint, WireStructure::Off, WireFloats::XorDelta, false).len();
    let memcpy = enc_mode(&rv, WireNumerics::Varint, WireStructure::Off, WireFloats::Memcpy, false).len();
    let postcard = postcard::to_allocvec(&series).unwrap().len();
    assert!(xor < memcpy, "XOR-delta must beat memcpy on a slow signal: {xor} vs {memcpy}");
    assert!(xor < postcard, "XOR-delta must beat postcard on a slow signal: {xor} vs {postcard}");
    println!(
        "\n── AXIS: TIME-SERIES floats (Gorilla XOR) ──\n  xor {xor}B  vs  memcpy {memcpy}B / postcard {postcard}B   ({:.2}× smaller)",
        memcpy as f64 / xor as f64
    );
}

// =====================================================================================
// RULE 5 — SHOW best compression: on repetitive data our toolbox crushes everyone.
// =====================================================================================
#[test]
fn we_crush_everyone_on_repetitive_data_with_compression() {
    let repetitive: Vec<i64> = (0..2000).map(|i| ((i % 8) as i64) * 1_000_000).collect();
    let ours = logos_best(&rv_ints(&repetitive));
    let postcard = postcard::to_allocvec(&repetitive).unwrap().len();
    let bincode = bincode::serialize(&repetitive).unwrap().len();
    let msgpack = rmp_serde::to_vec(&repetitive).unwrap().len();
    // The compression/dictionary knob makes us at least 10× smaller than the best of the
    // rest on highly-repetitive data (none of them compress).
    let best_rival = postcard.min(bincode).min(msgpack);
    assert!(
        ours * 10 < best_rival,
        "compression must make us >10× smaller on repetitive data: ours {ours} vs best rival {best_rival}"
    );
    println!(
        "\n── AXIS: COMPRESSION (repetitive data) ──\n  ours {ours}B  vs  best rival {best_rival}B   ({:.0}× smaller)",
        best_rival as f64 / ours as f64
    );
}

// =====================================================================================
// RULE 6 — BEAT the serde fixed-width codecs on DECODE (the fast-decode class capnp
//          also lives in). Our memcpy decode beats bincode's by a wide margin; the
//          capnp same-run comparison (it needs the capnp toolchain) lives in the
//          benchmark binary, which reports our fixed ~85ns vs capnp ~230ns.
// =====================================================================================
#[test]
fn we_beat_bincode_at_decode_speed() {
    let mut rng = Rng::new(0xDEC);
    let v: Vec<i64> = (0..1000).map(|_| rng.next() as i64).collect();
    let bytes = with_numerics(WireNumerics::Fixed, || enc(&rv_ints(&v)));
    let ours = batch_ns(50, 4000, || {
        black_box(message_from_wire(&bytes).unwrap());
    });
    let bin_bytes = bincode::serialize(&v).unwrap();
    let bincode = batch_ns(50, 4000, || {
        let out: Vec<i64> = bincode::deserialize(&bin_bytes).unwrap();
        black_box(out);
    });
    assert!(
        ours * 2.0 < bincode,
        "fixed decode must beat bincode by >2×: ours {ours:.0}ns vs bincode {bincode:.0}ns"
    );
}

// =====================================================================================
// RULE 7 — BEAT Cap'n Proto on its OWN axis: receive a message and read one field.
//          capnp's pitch is "zero-copy, cheap open." This refutes it on its own ground:
//          our `unframe` + aligned zero-copy `&[i64]` slice opens cheaper than capnp's
//          segment-table parse, and the element read after is a raw aligned-slice index —
//          identical machine code to capnp's primitive-list `get(i)`. Gated behind the
//          same `capnproto` feature as the bench (it needs the capnp compiler on PATH):
//          `cargo nextest run -p logicaffeine-wirebench --features capnproto`.
// =====================================================================================
#[cfg(feature = "capnproto")]
mod bench_capnp {
    include!(concat!(env!("OUT_DIR"), "/schemas/bench_capnp.rs"));
}

#[cfg(feature = "capnproto")]
#[test]
fn we_beat_capnp_on_receive_and_read_one_field() {
    use logicaffeine_compile::concurrency::marshal::{message_to_wire, view_message, with_struct_view};

    // A large `Int64` column — exactly the `List(Int64)` capnp is built for. The realistic
    // LAN / routing pattern: a message ARRIVES, you open it and read ONE element (a key or
    // index), then route or discard it — open is paid per message, not amortized.
    const N: usize = 4096;
    let data: Vec<i64> = (0..N as i64).map(|i| i.wrapping_mul(2_654_435_761)).collect();

    // Our aligned column (8-byte-aligned blob → a zero-copy `&[i64]`). Place it in an
    // 8-aligned backing, as a real zero-copy receiver holds pre-registered DMA buffers.
    let ours = with_struct_view(true, || message_to_wire("p", &rv_ints(&data)).unwrap());
    let mut backing = vec![0i64; ours.len() / 8 + 2];
    // SAFETY: copy the message into the aligned backing; thereafter read-only.
    unsafe {
        std::ptr::copy_nonoverlapping(ours.as_ptr(), backing.as_mut_ptr().cast::<u8>(), ours.len());
    }
    let ours_bytes: &[u8] = unsafe { std::slice::from_raw_parts(backing.as_ptr().cast::<u8>(), ours.len()) };

    // The capnp message (`Ints { v :List(Int64) }`).
    let mut b = capnp::message::Builder::new_default();
    {
        let root = b.init_root::<bench_capnp::ints::Builder>();
        let mut list = root.init_v(N as u32);
        for (i, &x) in data.iter().enumerate() {
            list.set(i as u32, x);
        }
    }
    let mut capnp_bytes = Vec::new();
    capnp::serialize::write_message(&mut capnp_bytes, &b).unwrap();

    // A deterministic index walk over the column.
    let idxs: Vec<usize> = (0..256).map(|k: usize| k.wrapping_mul(2_654_435_761) % N).collect();

    // Correctness gate: both read identical values at every index before we time anything.
    {
        let v = view_message(ours_bytes).unwrap();
        let s = v.as_i64_slice().expect("aligned zero-copy column");
        let mut slice = &capnp_bytes[..];
        let reader = capnp::serialize::read_message_from_flat_slice(&mut slice, capnp::message::ReaderOptions::new()).unwrap();
        let list = reader.get_root::<bench_capnp::ints::Reader>().unwrap().get_v().unwrap();
        for &i in &idxs {
            assert_eq!(s[i], list.get(i as u32), "same value at index {i}");
        }
    }

    // Same-run, min-of-rounds: each op = OPEN the received message + read one element.
    let mut k = 0usize;
    let ours_ns = batch_ns(80, 4000, || {
        let i = idxs[k % idxs.len()];
        k += 1;
        let v = view_message(black_box(ours_bytes)).unwrap();
        let s = v.as_i64_slice().unwrap();
        black_box(s[i]);
    });
    let mut k = 0usize;
    let capnp_ns = batch_ns(80, 4000, || {
        let i = idxs[k % idxs.len()];
        k += 1;
        let mut slice = black_box(&capnp_bytes[..]);
        let reader = capnp::serialize::read_message_from_flat_slice(&mut slice, capnp::message::ReaderOptions::new()).unwrap();
        let list = reader.get_root::<bench_capnp::ints::Reader>().unwrap().get_v().unwrap();
        black_box(list.get(i as u32));
    });

    eprintln!(
        "capnp open+read-one-field: ours {ours_ns:.0}ns vs capnp {capnp_ns:.0}ns ({:.2}× faster); size ours {} vs capnp {} B",
        capnp_ns / ours_ns,
        ours.len(),
        capnp_bytes.len()
    );
    assert!(
        ours_ns < capnp_ns,
        "open + read-one-field: ours {ours_ns:.0}ns must beat capnp {capnp_ns:.0}ns (cheaper open than capnp's segment-table parse)"
    );
    // And our framing overhead is no larger than capnp's (segment table + root struct word
    // + list pointer) — the payload itself is raw 8-byte in both, so this is a clean win.
    assert!(
        ours.len() <= capnp_bytes.len(),
        "our framing must be ≤ capnp's: ours {} vs capnp {}",
        ours.len(),
        capnp_bytes.len()
    );
}

#[cfg(feature = "capnproto")]
#[test]
fn we_beat_capnp_on_record_list_random_field_access() {
    use logicaffeine_compile::concurrency::marshal::{message_to_wire, view_message, with_struct_view};
    use logicaffeine_compile::interpreter::StructValue;
    use std::collections::HashMap;

    // A record LIST — capnp's `List(Record)` sweet spot. After receiving the message, read
    // ONE field of ONE row at a random index: capnp `items.get(i).get_id()` (compile-time
    // offsets) vs our `view.structs_row_field(i, "id")` (row + field offset tables). Both
    // O(1) random access; the open path is where the two differ.
    const N: usize = 2000;
    let names = ["alice", "bob", "carol", "dave", "erin"];
    let plain: Vec<(i64, &str, bool)> =
        (0..N).map(|i| (i as i64 * 7 + 1, names[i % names.len()], i % 2 == 0)).collect();

    // ours: a list of structs encoded as the record-list view (`T_STRUCTS_VIEW`).
    let rows: Vec<RuntimeValue> = plain
        .iter()
        .map(|(id, name, active)| {
            let mut f = HashMap::new();
            f.insert("id".to_string(), RuntimeValue::Int(*id));
            f.insert("name".to_string(), RuntimeValue::Text(Rc::new((*name).to_string())));
            f.insert("active".to_string(), RuntimeValue::Bool(*active));
            RuntimeValue::Struct(Box::new(StructValue { type_name: "Record".to_string(), fields: f }))
        })
        .collect();
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(rows))));
    let ours = with_struct_view(true, || message_to_wire("p", &v).unwrap());

    // capnp: `Records { items :List(Record) }`.
    let mut b = capnp::message::Builder::new_default();
    {
        let root = b.init_root::<bench_capnp::records::Builder>();
        let mut items = root.init_items(N as u32);
        for (i, (id, name, active)) in plain.iter().enumerate() {
            let mut rb = items.reborrow().get(i as u32);
            rb.set_id(*id);
            rb.set_name(name);
            rb.set_active(*active);
        }
    }
    let mut capnp_bytes = Vec::new();
    capnp::serialize::write_message(&mut capnp_bytes, &b).unwrap();

    let idxs: Vec<usize> = (0..256).map(|k: usize| k.wrapping_mul(2_654_435_761) % N).collect();

    // Correctness gate: both read identical ids at every sampled row.
    {
        let view = view_message(&ours).unwrap();
        let mut slice = &capnp_bytes[..];
        let reader =
            capnp::serialize::read_message_from_flat_slice(&mut slice, capnp::message::ReaderOptions::new()).unwrap();
        let items = reader.get_root::<bench_capnp::records::Reader>().unwrap().get_items().unwrap();
        for &i in &idxs {
            let ours_id = view.structs_row_field(i, "id").unwrap().as_int().unwrap();
            assert_eq!(ours_id, items.get(i as u32).get_id(), "same id at row {i}");
        }
    }

    let mut k = 0usize;
    let ours_ns = batch_ns(80, 4000, || {
        let i = idxs[k % idxs.len()];
        k += 1;
        let view = view_message(black_box(&ours)).unwrap();
        black_box(view.structs_row_field(i, "id").unwrap().as_int().unwrap());
    });
    let mut k = 0usize;
    let capnp_ns = batch_ns(80, 4000, || {
        let i = idxs[k % idxs.len()];
        k += 1;
        let mut slice = black_box(&capnp_bytes[..]);
        let reader =
            capnp::serialize::read_message_from_flat_slice(&mut slice, capnp::message::ReaderOptions::new()).unwrap();
        let items = reader.get_root::<bench_capnp::records::Reader>().unwrap().get_items().unwrap();
        black_box(items.get(i as u32).get_id());
    });

    eprintln!(
        "capnp record-list open+read-one-field: ours {ours_ns:.0}ns vs capnp {capnp_ns:.0}ns ({:.2}× faster); size ours {} vs capnp {} B",
        capnp_ns / ours_ns,
        ours.len(),
        capnp_bytes.len()
    );
    assert!(
        ours_ns < capnp_ns,
        "record-list open + read-one-field: ours {ours_ns:.0}ns must beat capnp {capnp_ns:.0}ns"
    );
}

// =====================================================================================
// RULE 8 — BEAT Cap'n Proto on END-TO-END LAN LATENCY. A real loopback TCP round-trip
//          (length-prefixed send → echo server → recv → read one field). The message is
//          pre-serialized for BOTH (capnp's "no serialize" best case, and our amortized
//          "serialize a snapshot once, send it" case), so this isolates the wire path:
//          fewer bytes through the kernel + a cheaper open. The win is driven by our proven
//          ~34% smaller record encoding, so on a big message the byte-copy dominates the
//          fixed syscall floor and the margin is robust. Gated behind `capnproto`.
// =====================================================================================
#[cfg(feature = "capnproto")]
#[test]
fn we_beat_capnp_on_lan_round_trip() {
    use logicaffeine_compile::concurrency::marshal::{message_to_wire, view_message, with_struct_view};
    use logicaffeine_compile::interpreter::StructValue;
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};

    // A big record list so the byte-copy on the wire dominates the fixed per-syscall cost.
    const N: usize = 20_000;
    let names = ["alice", "bob", "carol", "dave", "erin"];
    let plain: Vec<(i64, &str, bool)> =
        (0..N).map(|i| (i as i64 * 7 + 1, names[i % names.len()], i % 2 == 0)).collect();

    let rows: Vec<RuntimeValue> = plain
        .iter()
        .map(|(id, name, active)| {
            let mut f = HashMap::new();
            f.insert("id".to_string(), RuntimeValue::Int(*id));
            f.insert("name".to_string(), RuntimeValue::Text(Rc::new((*name).to_string())));
            f.insert("active".to_string(), RuntimeValue::Bool(*active));
            RuntimeValue::Struct(Box::new(StructValue { type_name: "Record".to_string(), fields: f }))
        })
        .collect();
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(rows))));
    let ours = with_struct_view(true, || message_to_wire("p", &v).unwrap());

    let mut b = capnp::message::Builder::new_default();
    {
        let root = b.init_root::<bench_capnp::records::Builder>();
        let mut items = root.init_items(N as u32);
        for (i, (id, name, active)) in plain.iter().enumerate() {
            let mut rb = items.reborrow().get(i as u32);
            rb.set_id(*id);
            rb.set_name(name);
            rb.set_active(*active);
        }
    }
    let mut capnp_bytes = Vec::new();
    capnp::serialize::write_message(&mut capnp_bytes, &b).unwrap();

    // A loopback echo server: read a length-prefixed frame, write it straight back. Runs
    // until the client disconnects (read_exact returns Err on EOF).
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        conn.set_nodelay(true).unwrap();
        let mut buf = Vec::new();
        loop {
            let mut len_buf = [0u8; 4];
            if conn.read_exact(&mut len_buf).is_err() {
                break;
            }
            let len = u32::from_be_bytes(len_buf) as usize;
            buf.resize(len, 0);
            if conn.read_exact(&mut buf).is_err() {
                break;
            }
            if conn.write_all(&len_buf).is_err() || conn.write_all(&buf).is_err() {
                break;
            }
        }
    });

    let mut client = TcpStream::connect(addr).unwrap();
    client.set_nodelay(true).unwrap();

    // One round-trip: send the frame, read the echo back into `buf`.
    let round_trip = |client: &mut TcpStream, msg: &[u8], buf: &mut Vec<u8>| {
        client.write_all(&(msg.len() as u32).to_be_bytes()).unwrap();
        client.write_all(msg).unwrap();
        let mut len_buf = [0u8; 4];
        client.read_exact(&mut len_buf).unwrap();
        let len = u32::from_be_bytes(len_buf) as usize;
        buf.resize(len, 0);
        client.read_exact(buf).unwrap();
    };

    // Correctness gate: the echo equals what we sent, and a field reads back correctly.
    let mut echo = Vec::new();
    round_trip(&mut client, &ours, &mut echo);
    assert_eq!(echo, ours, "ours echoes intact");
    assert_eq!(
        view_message(&echo).unwrap().structs_row_field(123, "id").unwrap().as_int().unwrap(),
        123 * 7 + 1,
        "field reads back from the echoed message"
    );
    round_trip(&mut client, &capnp_bytes, &mut echo);
    assert_eq!(echo, capnp_bytes, "capnp echoes intact");

    // Time the round-trip + read-one-field for each, min-of-rounds under the same load.
    let trials = 12u32;
    let iters = 200u32;
    let time_codec = |client: &mut TcpStream, msg: &[u8], read_id: &dyn Fn(&[u8]) -> i64| -> f64 {
        let mut echo = Vec::new();
        for _ in 0..iters {
            round_trip(client, msg, &mut echo); // warm up
        }
        let mut best = f64::MAX;
        for _ in 0..trials {
            let t = Instant::now();
            for _ in 0..iters {
                round_trip(client, msg, &mut echo);
                black_box(read_id(&echo));
            }
            best = best.min(t.elapsed().as_nanos() as f64 / iters as f64);
        }
        best
    };

    let ours_ns = time_codec(&mut client, &ours, &|bytes| {
        view_message(bytes).unwrap().structs_row_field(0, "id").unwrap().as_int().unwrap()
    });
    let capnp_ns = time_codec(&mut client, &capnp_bytes, &|bytes| {
        let mut slice = bytes;
        let reader =
            capnp::serialize::read_message_from_flat_slice(&mut slice, capnp::message::ReaderOptions::new()).unwrap();
        reader.get_root::<bench_capnp::records::Reader>().unwrap().get_items().unwrap().get(0).get_id()
    });

    drop(client); // disconnect → the server thread sees EOF and exits
    server.join().unwrap();

    eprintln!(
        "LAN loopback round-trip (N={N}): ours {:.1}µs vs capnp {:.1}µs ({:.2}× faster); on-wire ours {} vs capnp {} B ({:.0}% smaller)",
        ours_ns / 1000.0,
        capnp_ns / 1000.0,
        capnp_ns / ours_ns,
        ours.len(),
        capnp_bytes.len(),
        (1.0 - ours.len() as f64 / capnp_bytes.len() as f64) * 100.0
    );
    assert!(
        ours_ns < capnp_ns,
        "LAN round-trip: ours {ours_ns:.0}ns must beat capnp {capnp_ns:.0}ns (fewer bytes on the wire + cheaper open)"
    );
}
