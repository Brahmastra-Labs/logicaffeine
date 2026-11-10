use super::*;
use crate::interpreter::ClosureValue;
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// describe_columns — the codec naming its own output (the "which dial won" surface).
// ─────────────────────────────────────────────────────────────────────────────

fn ints_list(v: Vec<i64>) -> RuntimeValue {
    RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(v))))
}

#[test]
fn describe_columns_names_the_numeric_dials() {
    let ints = ints_list((0i64..64).map(|i| 3 + i * 7).collect());
    let enc = |num: WireNumerics| {
        with_structure(WireStructure::Off, || {
            with_numerics(num, || {
                message_to_wire_with("", &ints, WireCodec::Native, WireIntegrity::Raw).unwrap()
            })
        })
    };
    assert_eq!(describe_columns(&enc(WireNumerics::Varint)), vec!["varint"]);
    assert_eq!(
        describe_columns(&enc(WireNumerics::Fixed)),
        vec!["fixed (memcpy)"]
    );
    assert_eq!(
        describe_columns(&enc(WireNumerics::GroupVarint)),
        vec!["group-varint"]
    );
}

#[test]
fn describe_columns_names_the_affine_structure() {
    let ints = ints_list((0i64..64).map(|i| 5 + i * 3).collect());
    let bytes = with_structure(WireStructure::Affine, || {
        with_numerics(WireNumerics::Varint, || {
            message_to_wire_with("", &ints, WireCodec::Native, WireIntegrity::Raw).unwrap()
        })
    });
    assert_eq!(describe_columns(&bytes), vec!["affine (base,stride,n)"]);
}

#[test]
fn describe_columns_names_the_float_dials() {
    // Slowly-varying floats: memcpy stays raw; xor-delta shrinks (the dial applies).
    let floats = floats_list((0..256).map(|i| 20.0 + i as f64 * 0.01).collect());
    let enc = |fl: WireFloats| {
        with_structure(WireStructure::Off, || {
            with_floats(fl, || {
                message_to_wire_with("", &floats, WireCodec::Native, WireIntegrity::Raw).unwrap()
            })
        })
    };
    assert_eq!(
        describe_columns(&enc(WireFloats::Memcpy)),
        vec!["memcpy floats"]
    );
    assert_eq!(
        describe_columns(&enc(WireFloats::XorDelta)),
        vec!["xor-delta floats"]
    );
}

#[test]
fn describe_columns_names_strings_and_bools() {
    let strings = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(
        (0..24)
            .map(|i| RuntimeValue::Text(Rc::new(format!("host-{i}-{}", i * 31 % 7))))
            .collect(),
    ))));
    let bools = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(
        (0..40)
            .map(|i| RuntimeValue::Bool(i * 5 % 3 == 0))
            .collect(),
    ))));
    let enc = |rv: &RuntimeValue| {
        with_structure(WireStructure::Off, || {
            message_to_wire_with("", rv, WireCodec::Native, WireIntegrity::Raw).unwrap()
        })
    };
    assert_eq!(describe_columns(&enc(&strings)), vec!["flat strings"]);
    assert_eq!(describe_columns(&enc(&bools)), vec!["bit-packed bools"]);
}

#[test]
fn describe_columns_names_each_record_field() {
    let mut rows = Vec::new();
    for i in 0..32i64 {
        let mut f = HashMap::new();
        f.insert("id".to_string(), RuntimeValue::Int(i * 3 + 1));
        f.insert(
            "name".to_string(),
            RuntimeValue::Text(Rc::new(format!("node-{i}"))),
        );
        f.insert("active".to_string(), RuntimeValue::Bool(i % 2 == 0));
        rows.push(RuntimeValue::Struct(Box::new(StructValue {
            type_name: "Record".to_string(),
            fields: f,
        })));
    }
    let rv = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(rows))));
    let bytes = with_structure(WireStructure::Off, || {
        with_numerics(WireNumerics::Varint, || {
            message_to_wire_with("", &rv, WireCodec::Native, WireIntegrity::Raw).unwrap()
        })
    });
    // Field order is schema-defined; compare as a set so the test is order-robust.
    let cols = describe_columns(&bytes);
    let got: std::collections::BTreeSet<&str> = cols.iter().map(String::as_str).collect();
    let want: std::collections::BTreeSet<&str> = [
        "active: bit-packed bools",
        "id: varint",
        "name: flat strings",
    ]
    .into_iter()
    .collect();
    assert_eq!(got, want);
}

#[test]
fn column_tag_name_covers_the_structural_vocabulary() {
    // Every dial the codec can select must carry a plain-words name — none may ship as the
    // generic "value" fallback, or the benchmark card would hide which encoding actually won.
    for (tag, want) in [
        (T_INTS, "varint"),
        (T_INTS_FIXED, "fixed (memcpy)"),
        (T_INTS_GV, "group-varint"),
        (T_INTS_AFFINE, "affine (base,stride,n)"),
        (T_INTS_DELTA, "delta"),
        (T_INTS_DOD, "delta-of-delta"),
        (T_INTS_FOR, "FOR bit-pack"),
        (T_INTS_RLE, "run-length"),
        (T_INTS_DICT, "dictionary"),
        (T_FLOATS, "memcpy floats"),
        (T_FLOATS_XOR, "xor-delta floats"),
        (T_BOOLS, "bit-packed bools"),
        (T_STRINGS, "flat strings"),
    ] {
        assert_eq!(column_tag_name(tag), want, "tag {tag}");
        assert_ne!(
            column_tag_name(tag),
            "value",
            "tag {tag} must not be the generic fallback"
        );
    }
}

/// The wire-tag byte namespace is SHARED: `native_decode` dispatches one leading byte over BOTH
/// the marshal-local scalar/list tags AND the `describe::` int-column tags (routed through the
/// grouped arm). So a marshal scalar tag must never equal a `describe` column tag it does not
/// intentionally alias — otherwise a column whose menu picks that describe tag is mis-decoded as
/// the scalar (silent corruption / clean-`None` decode). This exact collision shipped once
/// (`T_COMPLEXF == describe::T_INTS_LRECUR == 82`), turning every linear-recurrence int column
/// on the wire into a bogus `ComplexF`. This lock makes that class of bug RED at test time.
#[test]
fn wire_tag_namespace_has_no_scalar_vs_column_collision() {
    use logicaffeine_base::describe;
    // Every `describe` column tag, from the table the leaf crate publishes — so a tag added there is
    // covered here without anyone remembering to list it.
    let externally_referenced_column_tags: &[(&str, u8)] = describe::COLUMN_TAGS;
    // Every marshal-local scalar/aggregate tag that decodes to a NON-column value — these are the
    // arms a colliding column tag would be stolen by.
    let scalar_tags: &[(&str, u8)] = &[
        ("T_NOTHING", T_NOTHING),
        ("T_FALSE", T_FALSE),
        ("T_TRUE", T_TRUE),
        ("T_INT", T_INT),
        ("T_WORD", T_WORD),
        ("T_BIGINT", T_BIGINT),
        ("T_RATIONAL", T_RATIONAL),
        ("T_DECIMAL", T_DECIMAL),
        ("T_COMPLEX", T_COMPLEX),
        ("T_COMPLEXF", T_COMPLEXF),
        ("T_MODULAR", T_MODULAR),
        ("T_QUANTITY", T_QUANTITY),
        ("T_MONEY", T_MONEY),
        ("T_UUID", T_UUID),
        ("T_FLOAT", T_FLOAT),
        ("T_CHAR", T_CHAR),
        ("T_TEXT", T_TEXT),
        ("T_DURATION", T_DURATION),
        ("T_DATE", T_DATE),
        ("T_MOMENT", T_MOMENT),
        ("T_SPAN", T_SPAN),
        ("T_TIME", T_TIME),
        ("T_PEER", T_PEER),
        ("T_LIST", T_LIST),
        ("T_TUPLE", T_TUPLE),
        ("T_SHARED_DEF", T_SHARED_DEF),
        ("T_SHARED_REF", T_SHARED_REF),
        ("T_EXACT", T_EXACT),
        ("T_SCALED", T_SCALED),
        ("T_DEPENDS", T_DEPENDS),
        ("T_CONVERGING", T_CONVERGING),
    ];
    for &(cname, ctag) in externally_referenced_column_tags {
        for &(sname, stag) in scalar_tags {
            assert_ne!(
                ctag, stag,
                "WIRE-TAG COLLISION: {cname} ({ctag}) == {sname} — a column that picks {cname} \
                     decodes as {sname}. Give the marshal scalar tag a value free in BOTH namespaces.",
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Build-in-place columnar records — Cap'n Proto's home turf (zero-encode + zero-decode).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn build_in_place_record_reads_back_zero_copy() {
    // Build a 1000-row record straight into the wire layout from borrowed slices (no
    // RuntimeValue), then read any column in O(1) and ZERO-COPY — `Some(slice)`, never the
    // copy fallback. The dual zero-encode/zero-decode story end to end.
    let ids: Vec<i64> = (0..1000).collect();
    let xs: Vec<f64> = (0..1000).map(|i| i as f64 * 0.5).collect();
    let bytes = build_columnar_record(
        "node",
        "Sensor",
        &[
            ("id", WireColumn::Ints(&ids)),
            ("x", WireColumn::Floats(&xs)),
        ],
    );
    let view = view_message(&bytes).expect("the built record opens as a view");
    let id_slice = view
        .struct_field("id")
        .expect("id field")
        .as_i64_slice()
        .expect("zero-copy i64");
    let x_slice = view
        .struct_field("x")
        .expect("x field")
        .as_f64_slice()
        .expect("zero-copy f64");
    assert_eq!(
        id_slice,
        &ids[..],
        "id column round-trips bit-exact, zero-copy"
    );
    assert_eq!(
        x_slice,
        &xs[..],
        "x column round-trips bit-exact, zero-copy"
    );
}

#[test]
fn build_in_place_is_byte_identical_to_the_runtimevalue_path() {
    // The builder emits EXACTLY the canonical struct-view bytes the audited `RuntimeValue`
    // encode path produces — so it inherits every correctness property of that path for free,
    // while skipping the value materialization + second serialize pass.
    let a: Vec<i64> = vec![10, 20, 30, 40];
    let b: Vec<f64> = vec![1.5, 2.5, 3.5];
    let built = build_columnar_record(
        "p",
        "Rec",
        &[
            ("alpha", WireColumn::Ints(&a)),
            ("beta", WireColumn::Floats(&b)),
        ],
    );

    let mut fields = HashMap::new();
    fields.insert(
        "alpha".to_string(),
        RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(a.clone())))),
    );
    fields.insert(
        "beta".to_string(),
        RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(b.clone())))),
    );
    let sv = RuntimeValue::Struct(Box::new(StructValue {
        type_name: "Rec".to_string(),
        fields,
    }));
    let canonical = with_struct_view(true, || {
        message_to_wire_with("p", &sv, WireCodec::Native, current_integrity()).unwrap()
    });
    assert_eq!(
        built, canonical,
        "build-in-place must equal the canonical struct-view encode byte-for-byte"
    );
}

#[test]
fn build_in_place_record_full_decode_interop() {
    // A non-view receiver (a full `message_from_wire` decode) reconstructs the record too —
    // the build-in-place form is ordinary wire bytes, not a view-only dialect.
    let a: Vec<i64> = vec![7, 8, 9];
    let bytes = build_columnar_record("p", "R", &[("c", WireColumn::Ints(&a))]);
    let (from, val) = message_from_wire(&bytes).expect("full decode");
    assert_eq!(from, "p");
    match val {
        RuntimeValue::Struct(s) => {
            assert_eq!(s.type_name, "R");
            match s.fields.get("c").unwrap() {
                RuntimeValue::List(l) => match &*l.borrow() {
                    ListRepr::Ints(v) => assert_eq!(v, &a),
                    other => panic!("expected Ints, got {other:?}"),
                },
                other => panic!("expected List, got {other:?}"),
            }
        }
        other => panic!("expected Struct, got {other:?}"),
    }
}

#[test]
#[ignore = "build-in-place encode-parity measurement — run with --ignored --nocapture"]
fn build_in_place_encode_is_at_capnp_parity() {
    // Honest measurement: Logos's column encode was ALREADY memcpy-fast (the aligned column is
    // one `extend_from_slice`), so build-in-place does NOT dramatically beat the existing path —
    // it MATCHES it (capnp parity on encode) while needing no intermediate `RuntimeValue`. The
    // comparison is fair: the value is pre-built once (the realistic "you already hold it" case),
    // so neither side pays a clone. The proven capnp *win* is the read side, not encode.
    use std::time::Instant;
    const ITERS: usize = 4000;
    let cols: Vec<Vec<i64>> = (0..8)
        .map(|c| (0..256).map(|i| (c * 256 + i) as i64).collect())
        .collect();
    let names: Vec<String> = (0..8).map(|c| format!("col{c}")).collect();

    // Pre-build the RuntimeValue ONCE — the existing path then only serializes (no clone).
    let mut fields_map = HashMap::new();
    for (n, c) in names.iter().zip(&cols) {
        fields_map.insert(
            n.clone(),
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(c.clone())))),
        );
    }
    let sv = RuntimeValue::Struct(Box::new(StructValue {
        type_name: "Batch".to_string(),
        fields: fields_map,
    }));

    // Each path is timed in rounds that alternate with the other's, after one untimed round of each: whichever
    // runs first in a fresh process otherwise pays its allocator and page warm-up, and a slower round from load
    // landing on one path cannot decide the comparison, since each path is judged by its best round. Both encode
    // without the frame's checksum: the FNV pass over the body costs the two paths the same and is most of a small
    // record's time, so with it on the comparison would not see the encodes at all.
    const ROUNDS: usize = 5;
    let in_place_round = || {
        let t = Instant::now();
        for _ in 0..ITERS {
            let fields: Vec<(&str, WireColumn)> = names
                .iter()
                .zip(&cols)
                .map(|(n, c)| (n.as_str(), WireColumn::Ints(c)))
                .collect();
            std::hint::black_box(with_integrity(WireIntegrity::Raw, || {
                build_columnar_record("p", "Batch", &fields)
            }));
        }
        t.elapsed()
    };
    let existing_round = || {
        let t = Instant::now();
        for _ in 0..ITERS {
            let bytes = with_struct_view(true, || {
                message_to_wire_with("p", &sv, WireCodec::Native, WireIntegrity::Raw).unwrap()
            });
            std::hint::black_box(bytes);
        }
        t.elapsed()
    };
    in_place_round();
    existing_round();
    let (mut in_place, mut serialize_existing) = (std::time::Duration::MAX, std::time::Duration::MAX);
    for _ in 0..ROUNDS {
        in_place = in_place.min(in_place_round());
        serialize_existing = serialize_existing.min(existing_round());
    }

    eprintln!(
        "encode 8×256 i64 record: build-in-place={in_place:?} serialize-existing={serialize_existing:?} ratio={:.2}x",
        serialize_existing.as_secs_f64() / in_place.as_secs_f64().max(f64::MIN_POSITIVE)
    );
    // Parity, not a fabricated win: build-in-place must be within a small band of the existing
    // memcpy-fast path (never meaningfully slower) — it ships the SAME bytes with no RuntimeValue.
    assert!(
        in_place.as_secs_f64() <= serialize_existing.as_secs_f64() * 1.5,
        "build-in-place must be at parity with the existing column encode: in_place={in_place:?} existing={serialize_existing:?}"
    );
}

#[test]
fn columnar_record_is_position_independent_mmap_and_ipc_ready() {
    // Cap'n Proto's FLAGSHIP: messages are position-independent (offset-based, never pointer-
    // based), so you can mmap a file or share a buffer across processes and read fields IN PLACE
    // — the OS pages in only what you touch, two processes share one segment with no kernel pipe.
    // `T_STRUCT_VIEW` uses RELATIVE offsets, so the SAME holds for Logos: the bytes read zero-copy
    // from ANY backing store at ANY base, with no relocation/fixup. This locks that property.
    let ids: Vec<i64> = (0..4096).collect();
    let xs: Vec<f64> = (0..4096).map(|i| i as f64 * 1.25).collect();
    let msg = build_columnar_record(
        "p",
        "Batch",
        &[
            ("id", WireColumn::Ints(&ids)),
            ("x", WireColumn::Floats(&xs)),
        ],
    );

    // Relocate the message to a different base address (an mmap maps at a page boundary; a shared
    // segment lands at its own offset). An 8-aligned shift keeps the columns zero-copy at the new
    // base; the read stays CORRECT at any base (position independence) — verified both ways.
    for &shift in &[0usize, 8, 16, 4096, 65536] {
        let mut arena = vec![0u8; shift];
        arena.extend_from_slice(&msg);
        let relocated = &arena[shift..]; // a fresh borrow at base + `shift`
        let view = view_message(relocated).expect("position-independent open at any base");
        let id_slice = view
            .struct_field("id")
            .unwrap()
            .as_i64_slice()
            .expect("zero-copy at aligned base");
        let x_slice = view
            .struct_field("x")
            .unwrap()
            .as_f64_slice()
            .expect("zero-copy at aligned base");
        assert_eq!(
            id_slice,
            &ids[..],
            "id column read in place at base+{shift}"
        );
        assert_eq!(x_slice, &xs[..], "x column read in place at base+{shift}");
    }

    // A NON-aligned base: the slice cast declines (alignment guard) — but the message is still
    // read correctly via the copy path, so correctness is base-independent, only the zero-copy
    // fast path needs alignment (which mmap/page boundaries always provide).
    let mut arena = vec![0u8; 3];
    arena.extend_from_slice(&msg);
    let view = view_message(&arena[3..]).expect("opens at an unaligned base too");
    let (_, val) = message_from_wire(&arena[3..]).expect("full decode at unaligned base");
    match val {
        RuntimeValue::Struct(s) => match s.fields.get("id").unwrap() {
            RuntimeValue::List(l) => match &*l.borrow() {
                ListRepr::Ints(v) => {
                    assert_eq!(v, &ids, "correct at an unaligned base via the copy path")
                }
                o => panic!("expected Ints, got {o:?}"),
            },
            o => panic!("expected List, got {o:?}"),
        },
        o => panic!("expected Struct, got {o:?}"),
    }
    // The field view still resolves at the unaligned base (offsets are relative); only the
    // zero-copy slice cast is alignment-gated.
    assert!(
        view.struct_field("id").is_some(),
        "field still locatable at an unaligned base"
    );
}

#[test]
fn columnar_record_mmaps_a_column_zero_copy_from_disk() {
    // The visceral crush of Cap'n Proto's headline: write the columnar record to a FILE, mmap
    // it, and read one column ZERO-COPY straight from the mapped pages — no parse, no decode, no
    // per-element copy. mmap pages start at a page boundary (4 KiB-aligned ⇒ 8-aligned), so the
    // aligned columns cast soundly. The OS pages in only what we touch. Smaller file than capnp
    // for the same data (name elision), and read in place all the same.
    use std::io::Write;
    let ids: Vec<i64> = (0..50_000).collect();
    let xs: Vec<f64> = (0..50_000).map(|i| (i as f64).sqrt()).collect();
    let msg = build_columnar_record(
        "p",
        "Telemetry",
        &[
            ("id", WireColumn::Ints(&ids)),
            ("x", WireColumn::Floats(&xs)),
        ],
    );

    let mut tmp = tempfile::NamedTempFile::new().expect("temp file");
    tmp.write_all(&msg).expect("write the wire message to disk");
    tmp.flush().expect("flush");
    let file = tmp.reopen().expect("reopen for mapping");
    // SAFETY: the file is not mutated while mapped (single-test, exclusive temp file).
    let mmap = unsafe { memmap2::Mmap::map(&file).expect("mmap the message file") };

    let view = view_message(&mmap[..]).expect("the mmap'd message opens in place");
    let id_slice = view
        .struct_field("id")
        .unwrap()
        .as_i64_slice()
        .expect("zero-copy i64 from mmap");
    let x_slice = view
        .struct_field("x")
        .unwrap()
        .as_f64_slice()
        .expect("zero-copy f64 from mmap");
    // The slices point INTO the mmap — no allocation, no decode. Verify against the source.
    assert_eq!(
        id_slice,
        &ids[..],
        "id column read zero-copy directly from the mmap"
    );
    assert_eq!(
        x_slice,
        &xs[..],
        "x column read zero-copy directly from the mmap"
    );
    // The slice genuinely aliases the mapped pages (zero-copy), not a decoded heap copy.
    let map_base = mmap.as_ptr() as usize;
    let slice_base = id_slice.as_ptr() as usize;
    assert!(
        slice_base >= map_base && slice_base < map_base + mmap.len(),
        "the i64 slice must alias the mapped pages, not a copy"
    );
}

#[test]
fn wireview_decode_and_schema_read_a_record_list_in_place() {
    // ZC1: a record-list view exposes its schema (type, fields, row count) and decodes any ONE
    // (row, field) cell in place — the primitives a lazy zero-copy receive backing reads through,
    // never decoding the rest of the list.
    let mk = |id: i64, x: f64| {
        let mut f = HashMap::new();
        f.insert("id".to_string(), RuntimeValue::Int(id));
        f.insert("x".to_string(), RuntimeValue::Float(x));
        RuntimeValue::Struct(Box::new(StructValue {
            type_name: "Rec".to_string(),
            fields: f,
        }))
    };
    let rows = vec![mk(10, 1.5), mk(20, 2.5), mk(30, 3.5)];
    let list = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(rows))));
    let bytes = with_struct_view(true, || {
        message_to_wire_with("p", &list, WireCodec::Native, WireIntegrity::Raw).unwrap()
    });
    let view = view_message(&bytes).expect("record list opens as a view");

    let (ty, fields, n) = view.structs_schema().expect("record-list schema in place");
    assert_eq!(ty, "Rec");
    assert_eq!(
        fields,
        vec!["id".to_string(), "x".to_string()],
        "sorted field schema"
    );
    assert_eq!(n, 3, "row count from the header, no rows decoded");

    assert_eq!(
        view.structs_row_field(1, "id").unwrap().decode(),
        Some(RuntimeValue::Int(20))
    );
    assert_eq!(
        view.structs_row_field(2, "x").unwrap().decode(),
        Some(RuntimeValue::Float(3.5))
    );
    assert_eq!(
        view.structs_row_field(0, "id").unwrap().decode(),
        Some(RuntimeValue::Int(10))
    );
    assert!(
        view.structs_row_field(3, "id").is_none(),
        "out-of-range row is None"
    );
    assert!(
        view.structs_row_field(0, "nope").is_none(),
        "missing field is None"
    );
}

#[test]
fn lazy_wirestructs_reads_records_without_eager_decode() {
    // ZC2: a received record-list held as RAW BYTES (ListRepr::WireStructs) reads any (row,
    // field) in place — `len` is O(1) with zero rows decoded, `get_field` touches one cell, and
    // full materialization matches the eager decode value-for-value.
    use crate::interpreter::{ListRepr, StructValue};
    let mk = |id: i64, x: f64, tag: &str| {
        let mut f = HashMap::new();
        f.insert("id".to_string(), RuntimeValue::Int(id));
        f.insert("x".to_string(), RuntimeValue::Float(x));
        f.insert(
            "tag".to_string(),
            RuntimeValue::Text(Rc::new(tag.to_string())),
        );
        RuntimeValue::Struct(Box::new(StructValue {
            type_name: "Rec".to_string(),
            fields: f,
        }))
    };
    let rows: Vec<RuntimeValue> = (0..1000)
        .map(|i| mk(i, i as f64 * 0.5, &format!("t{i}")))
        .collect();
    let eager = ListRepr::from_values(rows);
    let list = RuntimeValue::List(Rc::new(RefCell::new(eager.clone())));
    let bytes = with_struct_view(true, || {
        message_to_wire_with("p", &list, WireCodec::Native, WireIntegrity::Raw).unwrap()
    });

    let lazy = ListRepr::from_record_list_view(Rc::new(bytes)).expect("wraps as a lazy view");
    assert_eq!(
        lazy.len(),
        1000,
        "len is O(1) from the header — no rows decoded"
    );

    // Single-cell reads, located + decoded in place (never touching the other rows).
    assert_eq!(lazy.get_field(0, "id"), Some(RuntimeValue::Int(0)));
    assert_eq!(lazy.get_field(999, "id"), Some(RuntimeValue::Int(999)));
    assert_eq!(lazy.get_field(500, "x"), Some(RuntimeValue::Float(250.0)));
    match lazy.get_field(7, "tag") {
        Some(RuntimeValue::Text(s)) => assert_eq!(&*s, "t7"),
        o => panic!("expected tag text, got {o:?}"),
    }
    assert_eq!(lazy.get_field(0, "missing"), None, "missing field is None");

    // Whole-row reconstruction on demand.
    match lazy.get(3) {
        Some(RuntimeValue::Struct(s)) => {
            assert_eq!(s.type_name, "Rec");
            assert_eq!(s.fields.get("id"), Some(&RuntimeValue::Int(3)));
            assert_eq!(s.fields.get("x"), Some(&RuntimeValue::Float(1.5)));
        }
        o => panic!("expected struct row, got {o:?}"),
    }

    // Full materialization equals the eager decode, value-for-value.
    // Structural struct equality (the kernel's `values_equal` is reference-semantic for structs,
    // so compare type + every field by value).
    fn struct_eq(a: &RuntimeValue, b: &RuntimeValue) -> bool {
        match (a, b) {
            (RuntimeValue::Struct(x), RuntimeValue::Struct(y)) => {
                x.type_name == y.type_name
                    && x.fields.len() == y.fields.len()
                    && x.fields.iter().all(|(k, v)| {
                        y.fields
                            .get(k)
                            .is_some_and(|w| crate::semantics::compare::values_equal(v, w))
                    })
            }
            _ => crate::semantics::compare::values_equal(a, b),
        }
    }
    let lazy_vals = lazy.to_values();
    let eager_vals = eager.to_values();
    assert_eq!(lazy_vals.len(), eager_vals.len());
    for (idx, (a, b)) in lazy_vals.iter().zip(&eager_vals).enumerate() {
        assert!(
            struct_eq(a, b),
            "row {idx} differs:\n  lazy={a:?}\n eager={b:?}"
        );
    }
}

#[test]
fn message_from_wire_view_is_lazy_for_record_lists_eager_otherwise() {
    // ZC3: the lazy receive entry point holds a record list as raw bytes (WireStructs, NO rows
    // decoded) while any other shape full-decodes exactly as before. Sender is preserved either
    // way; the receiver opts in via the `view` knob (ZC4).
    use crate::interpreter::{ListRepr, StructValue};
    let mk = |id: i64| {
        let mut f = HashMap::new();
        f.insert("id".to_string(), RuntimeValue::Int(id));
        RuntimeValue::Struct(Box::new(StructValue {
            type_name: "R".to_string(),
            fields: f,
        }))
    };
    let list = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(
        (0..100).map(mk).collect(),
    ))));
    let bytes = with_struct_view(true, || {
        message_to_wire_with("alice", &list, WireCodec::Native, WireIntegrity::Raw).unwrap()
    });

    let (from, val) = message_from_wire_view(&bytes).expect("lazy view decode");
    assert_eq!(from, "alice", "sender preserved on the lazy path");
    match &val {
        RuntimeValue::List(rc) => {
            assert!(
                matches!(&*rc.borrow(), ListRepr::WireStructs { .. }),
                "a record list must be held LAZILY (no rows decoded), got {:?}",
                rc.borrow()
            );
            assert_eq!(rc.borrow().len(), 100, "O(1) len, no decode");
            assert_eq!(
                rc.borrow().get_field(42, "id"),
                Some(RuntimeValue::Int(42)),
                "in-place field read"
            );
        }
        o => panic!("expected a lazy list, got {o:?}"),
    }

    // A scalar message has no record-list view → full decode, identical to `message_from_wire`.
    let sbytes = message_to_wire_with(
        "bob",
        &RuntimeValue::Int(7),
        WireCodec::Native,
        WireIntegrity::Raw,
    )
    .unwrap();
    let (sfrom, sval) = message_from_wire_view(&sbytes).expect("scalar decode");
    assert_eq!(sfrom, "bob");
    assert_eq!(
        sval,
        RuntimeValue::Int(7),
        "non-record shape falls back to full decode"
    );
}

#[test]
fn production_receive_path_defers_then_reads_record_list_lazily() {
    // ZC5: mirror EXACTLY the interpreter's receive decisions for a record list:
    //   drain      → `peek_deferrable_sender` says "DEFER" (buffer raw, decode NOTHING)
    //   Await view  → `from_record_list_view` (LAZY — no rows decoded until touched)
    //   Await       → `message_from_wire` (eager) — same values
    // and prove a scalar is NOT deferrable (decoded in order at drain). This is the
    // "no decode in production" path, verified at the exact functions it calls.
    use crate::interpreter::{ListRepr, StructValue};
    let mk = |id: i64, name: &str| {
        let mut f = HashMap::new();
        f.insert("id".to_string(), RuntimeValue::Int(id));
        f.insert(
            "name".to_string(),
            RuntimeValue::Text(Rc::new(name.to_string())),
        );
        RuntimeValue::Struct(Box::new(StructValue {
            type_name: "User".to_string(),
            fields: f,
        }))
    };
    let rows: Vec<RuntimeValue> = (0..500).map(|i| mk(i, &format!("u{i}"))).collect();
    let list = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(rows))));
    let frame = with_struct_view(true, || {
        message_to_wire_with("alice", &list, WireCodec::Native, WireIntegrity::Raw).unwrap()
    });

    // 1) Drain: a record-list view is a DEFERRABLE message — peek yields the sender, no decode.
    assert_eq!(
        peek_deferrable_sender(&frame).as_deref(),
        Some("alice"),
        "record list defers at drain"
    );

    // 2) `Await view`: wrap the buffered frame LAZILY — a WireStructs, zero rows decoded.
    let lazy = ListRepr::from_record_list_view(Rc::new(frame.clone())).expect("lazy wrap");
    assert!(
        matches!(lazy, ListRepr::WireStructs { .. }),
        "Await view holds the list lazily"
    );
    assert_eq!(lazy.len(), 500, "O(1) len, nothing decoded");
    assert_eq!(
        lazy.get_field(123, "id"),
        Some(RuntimeValue::Int(123)),
        "in-place cell read"
    );

    // 3) `Await` (no view): the SAME buffered frame decodes eagerly to the same values.
    let (efrom, eager_val) = message_from_wire(&frame).expect("eager decode");
    assert_eq!(efrom, "alice");
    let eager_rows = match &eager_val {
        RuntimeValue::List(rc) => rc.borrow().to_values(),
        o => panic!("expected list, got {o:?}"),
    };
    let lazy_rows = lazy.to_values();
    assert_eq!(lazy_rows.len(), eager_rows.len());
    for (idx, (a, b)) in lazy_rows.iter().zip(&eager_rows).enumerate() {
        let eq = match (a, b) {
            (RuntimeValue::Struct(x), RuntimeValue::Struct(y)) => {
                x.type_name == y.type_name
                    && x.fields.iter().all(|(k, v)| {
                        y.fields
                            .get(k)
                            .is_some_and(|w| crate::semantics::compare::values_equal(v, w))
                    })
            }
            _ => false,
        };
        assert!(eq, "lazy and eager receive must agree at row {idx}");
    }

    // 4) A scalar message is NOT a deferrable record list → decoded eagerly in arrival order.
    let sframe = message_to_wire_with(
        "bob",
        &RuntimeValue::Int(7),
        WireCodec::Native,
        WireIntegrity::Raw,
    )
    .unwrap();
    assert_eq!(
        peek_deferrable_sender(&sframe),
        None,
        "a scalar is not deferred"
    );
}

#[test]
fn lazy_wirecolumn_reads_received_numeric_columns_zero_copy() {
    // EXTEND: a received aligned NUMERIC column (`Seq of Int`/`Seq of Float` sent fast/aligned)
    // is deferred at drain and read ZERO-COPY out of the borrowed `&[i64]`/`&[f64]` — capnp's
    // `List<i64>` read-in-place. `len` O(1), `get(i)` reads one element, no eager decode.
    use crate::interpreter::ListRepr;

    let ints: Vec<i64> = (0..2000).collect();
    let il = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(ints.clone()))));
    let ibytes = with_struct_view(true, || {
        message_to_wire_with("alice", &il, WireCodec::Native, WireIntegrity::Raw).unwrap()
    });
    assert_eq!(
        peek_deferrable_sender(&ibytes).as_deref(),
        Some("alice"),
        "aligned int column defers"
    );
    let lazy = ListRepr::from_received_view(Rc::new(ibytes)).expect("lazy int column");
    assert!(
        matches!(lazy, ListRepr::WireColumn { floats: false, .. }),
        "held as a lazy int column"
    );
    assert_eq!(lazy.len(), 2000, "O(1) len, no decode");
    assert_eq!(
        lazy.get(0),
        Some(RuntimeValue::Int(0)),
        "zero-copy element read"
    );
    assert_eq!(lazy.get(1999), Some(RuntimeValue::Int(1999)));
    assert_eq!(lazy.get(2000), None, "out of range");
    let vals = lazy.to_values();
    assert_eq!(vals.len(), 2000);
    assert_eq!(vals[500], RuntimeValue::Int(500));

    let floats: Vec<f64> = (0..1000).map(|i| i as f64 * 0.25).collect();
    let fl = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(floats.clone()))));
    let fbytes = with_struct_view(true, || {
        message_to_wire_with("bob", &fl, WireCodec::Native, WireIntegrity::Raw).unwrap()
    });
    assert_eq!(
        peek_deferrable_sender(&fbytes).as_deref(),
        Some("bob"),
        "aligned float column defers"
    );
    let lazyf = ListRepr::from_received_view(Rc::new(fbytes)).expect("lazy float column");
    assert!(
        matches!(lazyf, ListRepr::WireColumn { floats: true, .. }),
        "held as a lazy float column"
    );
    assert_eq!(lazyf.len(), 1000);
    assert_eq!(
        lazyf.get(4),
        Some(RuntimeValue::Float(1.0)),
        "zero-copy float read (0.25*4)"
    );
    assert_eq!(lazyf.to_values()[8], RuntimeValue::Float(2.0));
}

#[test]
fn batch_stream_message_round_trips() {
    // The `Stream`/`Await stream` substrate: many values batched into ONE framed blob, deframed
    // back in order, with the sender peekable and a normal message never mistaken for a stream.
    let values = vec![
        RuntimeValue::Int(1),
        RuntimeValue::Int(-42),
        RuntimeValue::Text(Rc::new("hi".to_string())),
        RuntimeValue::Bool(true),
    ];
    let blob = frame_stream_message("alice", &values).expect("frames a stream");
    assert_eq!(
        peek_stream_sender(&blob).as_deref(),
        Some("alice"),
        "sender peekable at drain"
    );

    let got = deframe_stream_message(&blob).expect("deframes the stream");
    assert_eq!(got.len(), 4);
    assert_eq!(got[0], RuntimeValue::Int(1));
    assert_eq!(got[1], RuntimeValue::Int(-42));
    match &got[2] {
        RuntimeValue::Text(t) => assert_eq!(&**t, "hi"),
        o => panic!("expected text, got {o:?}"),
    }
    assert_eq!(got[3], RuntimeValue::Bool(true));

    // A normal message is NOT a stream (the 0xFD marker disambiguates).
    let normal = message_to_wire("p", &RuntimeValue::Int(5)).unwrap();
    assert_eq!(
        peek_stream_sender(&normal),
        None,
        "a normal message is not a stream"
    );
    assert_eq!(deframe_stream_message(&normal), None);

    // An empty stream is valid (zero values).
    let empty = frame_stream_message("bob", &[]).unwrap();
    assert_eq!(peek_stream_sender(&empty).as_deref(), Some("bob"));
    assert_eq!(deframe_stream_message(&empty), Some(vec![]));
}

#[test]
fn build_in_place_edge_cases() {
    // Edge cases: an EMPTY column (still 8-aligned, reads back as `&[]`), a single element, and
    // a missing field (None, never a panic). The padding aligns even the zero-length blob.
    let empty: Vec<i64> = vec![];
    let one: Vec<i64> = vec![42];
    let bytes = build_columnar_record(
        "",
        "E",
        &[
            ("z", WireColumn::Ints(&empty)),
            ("o", WireColumn::Ints(&one)),
        ],
    );
    let view = view_message(&bytes).unwrap();
    assert_eq!(
        view.struct_field("z")
            .unwrap()
            .as_i64_slice()
            .expect("empty is still aligned"),
        &[] as &[i64]
    );
    assert_eq!(
        view.struct_field("o")
            .unwrap()
            .as_i64_slice()
            .expect("singleton zero-copy"),
        &[42]
    );
    assert!(
        view.struct_field("missing").is_none(),
        "a missing field is None, not a panic"
    );
}

/// A value round-trips iff materialize∘rebuild∘materialize is the identity on
/// the payload. We compare through `RtPayload` (which has structural
/// equality), because `RuntimeValue`'s `PartialEq` returns false for
/// collections/structs (reference semantics).
fn assert_roundtrips(v: &RuntimeValue) -> RtPayload {
    let p = materialize(v).expect("materialize");
    let back = rebuild(p.clone());
    let p2 = materialize(&back).expect("re-materialize");
    assert_eq!(p, p2, "marshalling round-trip changed the value");
    p
}

/// Encode `v` as an `Ints` list, decode it back, and return the recovered
/// integers — the round-trip oracle for the affine math hack.
fn affine_roundtrip(v: &[i64], s: WireStructure) -> (Vec<u8>, Vec<i64>) {
    let value = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(v.to_vec()))));
    let bytes = with_structure(s, || {
        message_to_wire_with("", &value, WireCodec::Native, WireIntegrity::Raw).unwrap()
    });
    let (_, back) = message_from_wire(&bytes).expect("decode");
    let got = match back {
        RuntimeValue::List(l) => match &*l.borrow() {
            ListRepr::Ints(g) => g.clone(),
            _ => panic!("expected an Ints list back"),
        },
        _ => panic!("expected a List back"),
    };
    (bytes, got)
}

#[test]
fn affine_int_column_elides_to_a_formula_and_round_trips() {
    let v: Vec<i64> = (0..1000).collect();
    let (bytes, got) = affine_roundtrip(&v, WireStructure::Affine);
    // The whole 1000-element column becomes (base, stride, n) — a handful of bytes.
    assert!(
        bytes.len() < 40,
        "a 1000-element affine column should elide to O(1) bytes; got {}",
        bytes.len()
    );
    assert_eq!(got, v, "affine round-trip must be exact");
}

#[test]
fn non_affine_column_is_not_elided_but_still_round_trips() {
    let mut v: Vec<i64> = (0..1000).collect();
    v[500] = 999_999; // break the progression — must NOT be mis-detected as affine
    let (bytes, got) = affine_roundtrip(&v, WireStructure::Affine);
    assert!(
        bytes.len() > 500,
        "a non-affine column must fall back to a real encoding; got {}",
        bytes.len()
    );
    assert_eq!(got, v, "fallback round-trip must be exact");
}

#[test]
fn affine_is_bijective_across_i64_overflow() {
    // A progression that wraps past i64::MAX — the wrapping match must reproduce it.
    let base = i64::MAX - 3;
    let stride = 5i64;
    let v: Vec<i64> = (0..100)
        .map(|i| base.wrapping_add((i as i64).wrapping_mul(stride)))
        .collect();
    let (_, got) = affine_roundtrip(&v, WireStructure::Affine);
    assert_eq!(got, v, "wrapping-affine round-trip must be exact");
}

// ---- G5: the per-column compression menu (WireStructure::Auto) -----------------

#[test]
fn wire_auto_delta_wins_on_monotone() {
    // A monotone column with small steps → delta makes the deltas one byte each.
    let v: Vec<i64> = (0..200i64)
        .scan(1000i64, |s, i| {
            *s += 1 + (i % 3);
            Some(*s)
        })
        .collect();
    let (auto, got) = affine_roundtrip(&v, WireStructure::Auto);
    let (varint, _) = affine_roundtrip(&v, WireStructure::Off);
    assert_eq!(got, v, "delta round-trips bit-exact");
    assert!(
        auto.len() < varint.len(),
        "Auto ({}) must beat varint ({}) on a monotone column",
        auto.len(),
        varint.len()
    );
}

#[test]
fn wire_auto_dod_wins_on_timestamps() {
    // Near-linear timestamps (large base + i·step + tiny jitter) → delta-of-delta ≈ 0.
    let v: Vec<i64> = (0..300i64)
        .map(|i| 1_700_000_000 + i * 1000 + (i % 5))
        .collect();
    let (auto, got) = affine_roundtrip(&v, WireStructure::Auto);
    let (varint, _) = affine_roundtrip(&v, WireStructure::Off);
    assert_eq!(got, v, "delta-of-delta round-trips bit-exact");
    assert!(
        auto.len() < varint.len(),
        "Auto ({}) must beat varint ({}) on timestamps",
        auto.len(),
        varint.len()
    );
}

#[test]
fn wire_auto_for_wins_on_clustered() {
    // A tight cluster around a large base → frame-of-reference bit-packs the residuals.
    let mut rng = SplitMix64 { state: 0x0000_F00D };
    let v: Vec<i64> = (0..400)
        .map(|_| 1_000_000 + (rng.next() % 16) as i64)
        .collect();
    let (auto, got) = affine_roundtrip(&v, WireStructure::Auto);
    let (varint, _) = affine_roundtrip(&v, WireStructure::Off);
    assert_eq!(got, v, "frame-of-reference round-trips bit-exact");
    assert!(
        auto.len() < varint.len(),
        "Auto ({}) must beat varint ({}) on a clustered column",
        auto.len(),
        varint.len()
    );
}

#[test]
fn wire_auto_polynomial_ships_the_generator_not_the_data() {
    // A degree-2 polynomial column ships a tiny GENERATOR (degree + a few seeds + n) —
    // "ship the computation, not the data" — and reconstructs bit-exact. The frontier
    // nobody else has: protobuf/capnp/arrow all ship the n raw values.
    let v: Vec<i64> = (0..10_000i64).map(|i| 3 * i * i - 5 * i + 7).collect();
    let (auto, got) = affine_roundtrip(&v, WireStructure::Auto);
    assert_eq!(got, v, "the polynomial column reconstructs bit-exact");
    let (varint, _) = affine_roundtrip(&v, WireStructure::Off);
    eprintln!(
        "polynomial generator: {} values → ours {} B vs raw varint {} B ({}× smaller)",
        v.len(),
        auto.len(),
        varint.len(),
        varint.len() / auto.len().max(1)
    );
    assert!(
        auto.len() < varint.len() / 100,
        "the generator ({} B) must ship ≪ the data ({} B)",
        auto.len(),
        varint.len()
    );
}

#[test]
fn wire_auto_polynomial_handles_cubic_and_negative_and_overflow() {
    // Degree-3 round-trips; a column whose differences overflow i64 falls back to the
    // menu (never mis-encodes); a non-polynomial column is left to the other candidates.
    let cubic: Vec<i64> = (0..500i64).map(|i| 2 * i * i * i - i * i + 11).collect();
    let (_, got) = affine_roundtrip(&cubic, WireStructure::Auto);
    assert_eq!(got, cubic, "cubic reconstructs bit-exact");

    let overflowing: Vec<i64> = vec![i64::MIN, i64::MAX, i64::MIN, i64::MAX];
    let (_, got) = affine_roundtrip(&overflowing, WireStructure::Auto);
    assert_eq!(
        got, overflowing,
        "an overflowing column still round-trips (via the menu)"
    );

    let mut rng = SplitMix64 { state: 0xBADC_0FFE };
    let noise: Vec<i64> = (0..500).map(|_| rng.next() as i64).collect();
    let (_, got) = affine_roundtrip(&noise, WireStructure::Auto);
    assert_eq!(
        got, noise,
        "random noise round-trips (no false polynomial detection)"
    );
}

#[test]
fn wire_auto_geometric_ships_the_generator_not_the_data() {
    // A geometric column `base * ratio^i` is NOT a polynomial (finite differences never
    // settle) and NOT affine, so without a dedicated detector it costs ~1 varint PER
    // element — the magnitude doubles every step. The generator ships THREE numbers
    // (base, ratio, count) regardless of length, and reconstructs bit-exact.
    let doubling: Vec<i64> = (0..40).map(|i| 3i64 * (1i64 << i)).collect();
    let (bytes, got) = affine_roundtrip(&doubling, WireStructure::Auto);
    assert_eq!(got, doubling, "a geometric column reconstructs bit-exact");
    assert!(
        bytes.len() < 20,
        "the geometric GENERATOR ships ~3 numbers, not 40 growing values: {} bytes",
        bytes.len()
    );

    // Negative ratio (alternating sign, growing magnitude) is geometric too.
    let alternating: Vec<i64> = {
        let mut c = 1i64;
        (0..40)
            .map(|_| {
                let v = c;
                c = c.wrapping_mul(-2);
                v
            })
            .collect()
    };
    let (bytes, got) = affine_roundtrip(&alternating, WireStructure::Auto);
    assert_eq!(
        got, alternating,
        "a negative-ratio geometric column reconstructs bit-exact"
    );
    assert!(
        bytes.len() < 20,
        "negative-ratio geometric also ships the generator: {} bytes",
        bytes.len()
    );

    // SOUNDNESS under overflow: a doubling sequence that runs PAST i64 wraps deterministically
    // — the detector verifies reproduction under the SAME `wrapping_mul` the decoder uses, so it
    // is still recognized AND round-trips bit-exact across the wrap (2^63 → i64::MIN → 0 → 0…).
    let wrapping: Vec<i64> = {
        let mut c = 1i64;
        (0..70)
            .map(|_| {
                let v = c;
                c = c.wrapping_mul(2);
                v
            })
            .collect()
    };
    let (_, got) = affine_roundtrip(&wrapping, WireStructure::Auto);
    assert_eq!(
        got, wrapping,
        "an overflowing geometric column still round-trips exactly"
    );

    // NO false positives: random noise, a near-geometric sequence with one perturbed element,
    // and an affine column are NOT mis-encoded as geometric — each round-trips exactly.
    let mut rng = SplitMix64 { state: 0x6E0_47E7 };
    let noise: Vec<i64> = (0..500).map(|_| rng.next() as i64).collect();
    let (_, got) = affine_roundtrip(&noise, WireStructure::Auto);
    assert_eq!(
        got, noise,
        "random noise round-trips (no false geometric detection)"
    );

    let mut perturbed: Vec<i64> = (0..30).map(|i| 5i64 * (1i64 << i)).collect();
    perturbed[17] += 1; // breaks the geometric law at one point
    let (_, got) = affine_roundtrip(&perturbed, WireStructure::Auto);
    assert_eq!(
        got, perturbed,
        "a perturbed near-geometric column round-trips (verification rejects it)"
    );

    let affine: Vec<i64> = (0..40).map(|i| 7 + 3 * i).collect();
    let (_, got) = affine_roundtrip(&affine, WireStructure::Auto);
    assert_eq!(
        got, affine,
        "an affine column round-trips (geometric does not steal it)"
    );
}

#[test]
fn wire_auto_periodic_ships_the_repeating_block() {
    // A cyclic column `pattern[i % p]` (weekly schedules, repeating categories, sawtooth
    // bytes) is none of affine/geometric/polynomial — but it is fully described by ONE
    // period's worth of values plus the count. Ship the block, not the 500 repeats.
    let block = [10i64, 20, 30, 40, 50];
    let cyclic: Vec<i64> = (0..500).map(|i| block[i % block.len()]).collect();
    let (bytes, got) = affine_roundtrip(&cyclic, WireStructure::Auto);
    assert_eq!(got, cyclic, "a periodic column reconstructs bit-exact");
    assert!(
        bytes.len() < 30,
        "the periodic GENERATOR ships ONE period ({} values), not 500: {} bytes",
        block.len(),
        bytes.len()
    );

    // Negative / mixed-magnitude period, and a non-trivial period length.
    let block2 = [-7i64, 0, 1000000, -3, 42, 42, -1];
    let cyclic2: Vec<i64> = (0..1001).map(|i| block2[i % block2.len()]).collect();
    let (bytes, got) = affine_roundtrip(&cyclic2, WireStructure::Auto);
    assert_eq!(
        got, cyclic2,
        "a mixed-magnitude periodic column reconstructs bit-exact"
    );
    assert!(
        bytes.len() < 40,
        "still ships ONE period, not 1001 values: {} bytes",
        bytes.len()
    );

    // Minimal period wins: a column that is ALSO period-10 (because it is period-5) ships the
    // SMALLER period-5 block — and round-trips.
    let p5: Vec<i64> = (0..200).map(|i| block[i % 5]).collect();
    let (_, got) = affine_roundtrip(&p5, WireStructure::Auto);
    assert_eq!(got, p5, "the minimal period round-trips");

    // NO false positives: random noise and an aperiodic column (period == length, no repeat)
    // are NOT mis-encoded as periodic — each round-trips exactly.
    let mut rng = SplitMix64 { state: 0x9E15_AB0 };
    let noise: Vec<i64> = (0..500).map(|_| rng.next() as i64).collect();
    let (_, got) = affine_roundtrip(&noise, WireStructure::Auto);
    assert_eq!(
        got, noise,
        "random noise round-trips (no false periodic detection)"
    );

    let aperiodic: Vec<i64> = (0..50).map(|i| i * i + 1).collect();
    let (_, got) = affine_roundtrip(&aperiodic, WireStructure::Auto);
    assert_eq!(got, aperiodic, "an aperiodic column round-trips");
}

#[test]
fn wire_float_default_dial_is_pure_memcpy() {
    // The lightning-quick contract: under the DEFAULT (`Off`) dial the float encoder never
    // inspects the data — even a constant / affine / periodic column ships as the raw n×8 memcpy
    // (`T_FLOATS`). Structural shrinking is opt-in via `Auto`/`Affine`. This mirrors the integer
    // contract (Off = straight varint, no detection) and is what keeps the hot encode path fast:
    // you pick the dial ahead of time, the encoder does not search on every send.
    let shapes: Vec<(&str, Vec<f64>)> = vec![
        ("constant", vec![3.14159f64; 1000]),
        ("affine", (0..1000).map(|i| i as f64).collect()),
        (
            "periodic",
            (0..1000)
                .map(|i| [1.5f64, -2.25, 3.0, 0.0, 99.99][i % 5])
                .collect(),
        ),
    ];
    for (name, v) in &shapes {
        let val = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(v.clone()))));
        let bytes = message_to_wire_with("", &val, WireCodec::Native, WireIntegrity::Raw).unwrap();
        // The tag is the plain memcpy form and the body carries all n×8 raw bytes — NOT a tiny
        // generator. (Framing adds a few bytes on top, so `>=` n×8 is the memcpy signature.)
        assert!(
            bytes.len() >= v.len() * 8,
            "[{name}] the Off dial must ship the raw memcpy (≥ {} B), got {} B (detection leaked into the hot path)",
            v.len() * 8,
            bytes.len()
        );
        let got = match message_from_wire(&bytes).unwrap().1 {
            RuntimeValue::List(l) => match &*l.borrow() {
                ListRepr::Floats(g) => g.clone(),
                _ => panic!("expected Floats"),
            },
            _ => panic!("expected List"),
        };
        assert_eq!(
            &got, v,
            "[{name}] memcpy float column round-trips bit-exact"
        );
    }
    // The opt-in still works: the SAME constant column under `Auto` ships the tiny generator.
    let c = vec![3.14159f64; 1000];
    let val = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(c))));
    let small = with_structure(WireStructure::Auto, || {
        message_to_wire_with("", &val, WireCodec::Native, WireIntegrity::Raw).unwrap()
    });
    assert!(
        small.len() < 30,
        "Auto still ships the generator (one f64 + count): {} B",
        small.len()
    );
}

#[test]
fn wire_float_const_and_affine_ship_the_generator() {
    // Ship the GENERATOR for floats too: a constant column = one f64 + count; a bit-exact
    // `base + i·stride` column = three numbers. Both lossless; real noisy data falls through.
    fn roundtrip(v: Vec<f64>) -> (Vec<u8>, Vec<f64>) {
        let val = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(v))));
        // Structural float forms are `Auto`/`Affine`-only now (the default dial is a pure memcpy —
        // see `wire_float_default_dial_is_pure_memcpy`); opt into the menu to exercise them.
        let bytes = with_structure(WireStructure::Auto, || {
            message_to_wire_with("", &val, WireCodec::Native, WireIntegrity::Raw).unwrap()
        });
        let got = match message_from_wire(&bytes).unwrap().1 {
            RuntimeValue::List(l) => match &*l.borrow() {
                ListRepr::Floats(g) => g.clone(),
                _ => panic!("expected Floats"),
            },
            _ => panic!("expected List"),
        };
        (bytes, got)
    }

    // Constant — 1000 identical f64 ship in ~11 bytes.
    let c = vec![3.14159f64; 1000];
    let (bytes, got) = roundtrip(c.clone());
    assert_eq!(got, c, "constant float column reconstructs");
    assert!(
        bytes.len() < 30,
        "constant ships one f64 + count, not 8000 B: {} B",
        bytes.len()
    );

    // Affine — integer-valued floats (exact in f64) ship as base+stride+count.
    let ints: Vec<f64> = (0..1000).map(|i| i as f64).collect();
    let (bytes, got) = roundtrip(ints.clone());
    assert_eq!(
        got, ints,
        "integer-valued float column reconstructs bit-exact"
    );
    assert!(
        bytes.len() < 40,
        "affine ships 3 numbers, not 8000 B: {} B",
        bytes.len()
    );

    // Affine with a power-of-two stride — `i·0.5` is bit-exact.
    let half: Vec<f64> = (0..500).map(|i| (i as f64) * 0.5).collect();
    let (bytes, got) = roundtrip(half.clone());
    assert_eq!(
        got, half,
        "power-of-two-stride affine reconstructs bit-exact"
    );
    assert!(bytes.len() < 40, "still 3 numbers: {} B", bytes.len());

    // NO false positives: noisy finite floats and a perturbed near-affine column round-trip
    // exactly via the raw/XOR path (the bit-exact check refuses anything that isn't perfect).
    let mut rng = SplitMix64 {
        state: 0xF10A7_C0DE,
    };
    let noise: Vec<f64> = (0..500)
        .map(|_| (rng.next() % 10_000_000) as f64 / 13.0)
        .collect();
    let (_, got) = roundtrip(noise.clone());
    assert_eq!(
        got, noise,
        "noisy floats round-trip (no false generator detection)"
    );

    let mut perturbed: Vec<f64> = (0..50).map(|i| i as f64).collect();
    perturbed[37] = 36.9999999;
    let (_, got) = roundtrip(perturbed.clone());
    assert_eq!(got, perturbed, "a perturbed near-affine column round-trips");
}

#[test]
fn wire_auto_sparse_column_ships_dominant_plus_exceptions() {
    // A mostly-one-value column with a handful of DIVERSE-valued exceptions ships the dominant
    // value + a short (delta-index, value) list — beating dict (which would bit-pack 1000 indices
    // over 11 distinct values) and RLE (two run-entries per isolated exception).
    let mut v = vec![0i64; 1000];
    for k in 0..10 {
        v[k * 97] = (k as i64 + 1) * 12345;
    }
    let (bytes, got) = affine_roundtrip(&v, WireStructure::Auto);
    assert_eq!(got, v, "sparse column reconstructs exactly");
    assert!(
        bytes.len() < 80,
        "sparse ships ~10 exceptions, not 1000 values: {} bytes",
        bytes.len()
    );

    // A non-zero dominant value works too.
    let mut v2 = vec![42i64; 500];
    for k in 0..5 {
        v2[k * 80 + 3] = -(k as i64 + 1) * 7_000_003;
    }
    let (bytes, got) = affine_roundtrip(&v2, WireStructure::Auto);
    assert_eq!(
        got, v2,
        "non-zero-dominant sparse column reconstructs exactly"
    );
    assert!(
        bytes.len() < 80,
        "still ~5 exceptions: {} bytes",
        bytes.len()
    );

    // NO false positives: a column with NO dominant value (random) round-trips via the menu,
    // not mis-encoded as sparse.
    let mut rng = SplitMix64 {
        state: 0x5A95E_C0DE,
    };
    let noise: Vec<i64> = (0..500).map(|_| rng.next() as i64).collect();
    let (_, got) = affine_roundtrip(&noise, WireStructure::Auto);
    assert_eq!(
        got, noise,
        "random column round-trips (no false sparse detection)"
    );
}

#[test]
fn wire_float_sparse_column_ships_dominant_plus_exceptions() {
    // The float twin of sparse: a mostly-one-value f64 column (sparse telemetry, a mostly-zero
    // signal with a few spikes) ships the dominant value + the outliers, not all n×8 bytes.
    fn roundtrip(v: Vec<f64>) -> (Vec<u8>, Vec<f64>) {
        let val = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(v))));
        // Structural float forms are `Auto`/`Affine`-only now (the default dial is a pure memcpy —
        // see `wire_float_default_dial_is_pure_memcpy`); opt into the menu to exercise them.
        let bytes = with_structure(WireStructure::Auto, || {
            message_to_wire_with("", &val, WireCodec::Native, WireIntegrity::Raw).unwrap()
        });
        let got = match message_from_wire(&bytes).unwrap().1 {
            RuntimeValue::List(l) => match &*l.borrow() {
                ListRepr::Floats(g) => g.clone(),
                _ => panic!("expected Floats"),
            },
            _ => panic!("expected List"),
        };
        (bytes, got)
    }

    let mut v = vec![0.0f64; 1000];
    for k in 0..10 {
        v[k * 97] = (k as f64 + 1.0) * 1234.5;
    }
    let (bytes, got) = roundtrip(v.clone());
    assert_eq!(got, v, "sparse float column reconstructs exactly");
    assert!(
        bytes.len() < 160,
        "sparse ships ~10 outliers, not 8000 B: {} B",
        bytes.len()
    );

    // Non-zero dominant, with negative outliers.
    let mut v2 = vec![3.5f64; 500];
    for k in 0..5 {
        v2[k * 80 + 3] = -(k as f64 + 1.0) * 99.0;
    }
    let (_, got) = roundtrip(v2.clone());
    assert_eq!(
        got, v2,
        "non-zero-dominant sparse float column reconstructs exactly"
    );

    // No dominant value (random finite floats) → not mis-encoded; round-trips via memcpy.
    let mut rng = SplitMix64 {
        state: 0xF10A7_5A95E,
    };
    let noise: Vec<f64> = (0..500)
        .map(|_| (rng.next() % 10_000_000) as f64 / 13.0)
        .collect();
    let (_, got) = roundtrip(noise.clone());
    assert_eq!(
        got, noise,
        "random float column round-trips (no false sparse detection)"
    );
}

#[test]
fn wire_float_periodic_and_geometric_ship_the_generator() {
    fn roundtrip(v: Vec<f64>) -> (Vec<u8>, Vec<f64>) {
        let val = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(v))));
        // Structural float forms are `Auto`/`Affine`-only now (the default dial is a pure memcpy —
        // see `wire_float_default_dial_is_pure_memcpy`); opt into the menu to exercise them.
        let bytes = with_structure(WireStructure::Auto, || {
            message_to_wire_with("", &val, WireCodec::Native, WireIntegrity::Raw).unwrap()
        });
        let got = match message_from_wire(&bytes).unwrap().1 {
            RuntimeValue::List(l) => match &*l.borrow() {
                ListRepr::Floats(g) => g.clone(),
                _ => panic!("expected Floats"),
            },
            _ => panic!("expected List"),
        };
        (bytes, got)
    }

    // Periodic — a repeating 5-float waveform ships ONE block, not 1000 samples.
    let block = [1.5f64, -2.25, 3.0, 0.0, 99.99];
    let cyclic: Vec<f64> = (0..1000).map(|i| block[i % 5]).collect();
    let (bytes, got) = roundtrip(cyclic.clone());
    assert_eq!(got, cyclic, "periodic float column reconstructs");
    assert!(
        bytes.len() < 80,
        "ships one 5-float block, not 1000: {} B",
        bytes.len()
    );

    // Geometric — doubling and halving (power-of-two ratios) are bit-exact in f64.
    let doubling: Vec<f64> = {
        let mut c = 1.0f64;
        (0..50)
            .map(|_| {
                let x = c;
                c *= 2.0;
                x
            })
            .collect()
    };
    let (bytes, got) = roundtrip(doubling.clone());
    assert_eq!(
        got, doubling,
        "doubling float column reconstructs bit-exact"
    );
    assert!(
        bytes.len() < 40,
        "geometric ships base+ratio+count: {} B",
        bytes.len()
    );

    let halving: Vec<f64> = {
        let mut c = 1024.0f64;
        (0..40)
            .map(|_| {
                let x = c;
                c *= 0.5;
                x
            })
            .collect()
    };
    let (_, got) = roundtrip(halving.clone());
    assert_eq!(
        got, halving,
        "halving (exponential decay) reconstructs bit-exact"
    );

    // NO false positives: random finite floats are neither periodic nor geometric — round-trip exact.
    let mut rng = SplitMix64 {
        state: 0xC0FFEE_F10A7,
    };
    let noise: Vec<f64> = (0..500)
        .map(|_| (rng.next() % 9_999_991) as f64 / 7.0 - 1234.5)
        .collect();
    let (_, got) = roundtrip(noise.clone());
    assert_eq!(
        got, noise,
        "random floats round-trip (no false periodic/geometric detection)"
    );
}

#[test]
fn wire_string_template_ships_prefix_suffix_and_affine_index() {
    // Sequential-id strings — REST URLs, file paths, generated labels — are `prefix + (base +
    // i·stride) + suffix`. Ship the two affixes once + the affine index, not all n strings.
    fn roundtrip(strings: &[String]) -> (Vec<u8>, Vec<String>) {
        let items: Vec<RuntimeValue> = strings
            .iter()
            .map(|s| RuntimeValue::Text(Rc::new(s.clone())))
            .collect();
        let val = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(items))));
        let bytes = with_structure(WireStructure::Auto, || {
            message_to_wire_with("", &val, WireCodec::Native, WireIntegrity::Raw).unwrap()
        });
        let got = match message_from_wire(&bytes).unwrap().1 {
            RuntimeValue::List(l) => {
                let b = l.borrow();
                (0..b.len())
                    .map(|i| match b.get(i).unwrap() {
                        RuntimeValue::Text(s) => (*s).clone(),
                        other => panic!("expected Text, got {other:?}"),
                    })
                    .collect()
            }
            _ => panic!("expected List"),
        };
        (bytes, got)
    }

    // Sequential-id URLs (long shared prefix) → ~40 bytes, not 37 KB.
    let urls: Vec<String> = (0..1000)
        .map(|i| format!("https://api.example.com/v1/items/{i}"))
        .collect();
    let (bytes, got) = roundtrip(&urls);
    assert_eq!(got, urls, "templated URLs reconstruct exactly");
    assert!(
        bytes.len() < 80,
        "ships prefix + affine index, not 1000 URLs: {} bytes",
        bytes.len()
    );

    // Non-unit stride, and a prefix+suffix template (`file_<i>.txt`).
    let stepped: Vec<String> = (0..500).map(|i| format!("row_{}", i * 2)).collect();
    let (_, got) = roundtrip(&stepped);
    assert_eq!(
        got, stepped,
        "stride-2 templated labels reconstruct exactly"
    );

    let files: Vec<String> = (0..300).map(|i| format!("file_{i}.txt")).collect();
    let (bytes, got) = roundtrip(&files);
    assert_eq!(got, files, "prefix+suffix template reconstructs exactly");
    assert!(
        bytes.len() < 60,
        "prefix+suffix template stays tiny: {} bytes",
        bytes.len()
    );

    // NO false positives: non-affine ids and ZERO-PADDED ids (exact-decimal check) round-trip
    // via the flat/dictionary path, never a wrong template.
    let mut rng = SplitMix64 {
        state: 0x57117_C0DE,
    };
    let scattered: Vec<String> = (0..200)
        .map(|_| format!("k{}", rng.next() % 1_000_000))
        .collect();
    let (_, got) = roundtrip(&scattered);
    assert_eq!(
        got, scattered,
        "non-affine ids round-trip (no false template)"
    );

    let padded: Vec<String> = (0..50).map(|i| format!("id_{i:03}")).collect();
    let (_, got) = roundtrip(&padded);
    assert_eq!(
        got, padded,
        "zero-padded ids round-trip (not templated — exact-decimal guard)"
    );
}

#[test]
fn wire_string_front_coding_crushes_sorted_shared_prefix_columns() {
    // The structural string compressor the dictionary (all-distinct → no win) and template
    // (non-affine / zero-padded → bails) can't touch: a sorted or hierarchical column whose
    // adjacent strings share long prefixes ships each as (shared-prefix-len, suffix).
    fn roundtrip(strings: &[String]) -> (Vec<u8>, Vec<String>) {
        let items: Vec<RuntimeValue> = strings
            .iter()
            .map(|s| RuntimeValue::Text(Rc::new(s.clone())))
            .collect();
        let val = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(items))));
        let bytes = with_structure(WireStructure::Auto, || {
            message_to_wire_with("", &val, WireCodec::Native, WireIntegrity::Raw).unwrap()
        });
        let got = match message_from_wire(&bytes).unwrap().1 {
            RuntimeValue::List(l) => {
                let b = l.borrow();
                (0..b.len())
                    .map(|i| match b.get(i).unwrap() {
                        RuntimeValue::Text(s) => (*s).clone(),
                        other => panic!("expected Text, got {other:?}"),
                    })
                    .collect()
            }
            _ => panic!("expected List"),
        };
        (bytes, got)
    }
    let flat_len = |v: &[String]| -> usize { v.iter().map(|s| s.len()).sum() };

    // Zero-padded log paths (long shared prefix AND suffix; template bails on the zero-padding,
    // the dictionary can't help 500 distinct strings — front-coding crushes them).
    let paths: Vec<String> = (0..500)
        .map(|i| format!("/var/log/app/2026/06/service-{i:04}.log"))
        .collect();
    let (bytes, got) = roundtrip(&paths);
    assert_eq!(got, paths, "front-coded log paths reconstruct exactly");
    assert!(
        bytes.len() * 3 < flat_len(&paths),
        "front-coding crushes the shared path prefix: {} vs flat {}",
        bytes.len(),
        flat_len(&paths)
    );

    // Sorted hierarchical object-store keys — non-affine, all distinct, deep shared prefixes.
    let mut keys: Vec<String> = Vec::new();
    for user in ["alice", "bob", "carol", "dave"] {
        for kind in ["profile", "settings", "avatar", "session"] {
            keys.push(format!("users/{user}/{kind}/data.json"));
        }
    }
    keys.sort();
    let (bytes, got) = roundtrip(&keys);
    assert_eq!(
        got, keys,
        "front-coded hierarchical keys reconstruct exactly"
    );
    assert!(
        bytes.len() < flat_len(&keys),
        "front-coding beats flat on hierarchical keys"
    );

    // MULTI-BYTE UTF-8 shared prefix: the `café` prefix (é = 2 bytes) must be cut on a char
    // boundary so the suffix stays valid UTF-8 — round-trips bit-exact.
    let unicode: Vec<String> = (0..100)
        .map(|i| format!("café/naïve/Москва/key-{i:03}"))
        .collect();
    let (_, got) = roundtrip(&unicode);
    assert_eq!(
        got, unicode,
        "front-coding cuts on UTF-8 char boundaries (no corruption)"
    );

    // NO false win / NO corruption on a column with NO shared prefixes (high-entropy, unsorted):
    // `consider` keeps flat, and it still round-trips.
    let mut rng = SplitMix64 { state: 0xF20D_C0DE };
    let scattered: Vec<String> = (0..200)
        .map(|_| {
            let len = 6 + (rng.next() % 10) as usize;
            (0..len)
                .map(|_| (b'a' + (rng.next() % 26) as u8) as char)
                .collect()
        })
        .collect();
    let (_, got) = roundtrip(&scattered);
    assert_eq!(
        got, scattered,
        "prefix-free random strings round-trip (front-coding not falsely applied)"
    );
}

#[test]
fn wire_bool_generators_ship_constant_alternating_and_run_columns() {
    // The generator theme completed for the LAST column type: a constant / alternating / cyclic
    // bool column ships its tiny period, a run-structured one ships its runs — none of which the
    // 1-bit bit-pack (≈ n/8 bytes) can touch — while a random column correctly falls back.
    fn roundtrip(v: &[bool]) -> (Vec<u8>, Vec<bool>) {
        let val = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Bools(v.to_vec()))));
        let bytes = with_structure(WireStructure::Auto, || {
            message_to_wire_with("", &val, WireCodec::Native, WireIntegrity::Raw).unwrap()
        });
        let got = match message_from_wire(&bytes).unwrap().1 {
            RuntimeValue::List(l) => match &*l.borrow() {
                ListRepr::Bools(b) => b.clone(),
                other => panic!("expected Bools, got {other:?}"),
            },
            _ => panic!("expected List"),
        };
        (bytes, got)
    }

    // Constant columns (all-true / all-false) → period-1 generator, not 125 packed bytes.
    for constant in [true, false] {
        let col = vec![constant; 1000];
        let (bytes, got) = roundtrip(&col);
        assert_eq!(got, col, "constant bool column reconstructs exactly");
        assert!(
            bytes.len() < 20,
            "constant={constant} ships a period-1 generator: {} bytes",
            bytes.len()
        );
    }

    // Alternating → period-2; a weekly (period-7) flag → period-7.
    let alt: Vec<bool> = (0..1000).map(|i| i % 2 == 0).collect();
    let (bytes, got) = roundtrip(&alt);
    assert_eq!(got, alt, "alternating reconstructs exactly");
    assert!(
        bytes.len() < 20,
        "alternating ships a period-2 generator: {} bytes",
        bytes.len()
    );

    let weekly: Vec<bool> = (0..700)
        .map(|i| [true, true, true, true, true, false, false][i % 7])
        .collect();
    let (bytes, got) = roundtrip(&weekly);
    assert_eq!(got, weekly, "weekly flag reconstructs exactly");
    assert!(
        bytes.len() < 20,
        "period-7 weekly flag ships as periodic: {} bytes",
        bytes.len()
    );

    // Two big runs ([F×500, T×500]) have NO short period → RLE crushes them.
    let runs: Vec<bool> = (0..1000).map(|i| i >= 500).collect();
    let (bytes, got) = roundtrip(&runs);
    assert_eq!(got, runs, "two big runs reconstruct exactly");
    assert!(
        bytes.len() < 20,
        "two big runs ship as RLE: {} bytes",
        bytes.len()
    );

    // Mostly-true with a few scattered flips → RLE beats the 125-byte bit-pack.
    let mut mostly = vec![true; 1000];
    for k in [37, 199, 450, 777, 988] {
        mostly[k] = false;
    }
    let (bytes, got) = roundtrip(&mostly);
    assert_eq!(got, mostly, "mostly-true-with-flips reconstructs exactly");
    assert!(
        bytes.len() < 60,
        "a handful of flips ships as RLE, not 125 packed bytes: {} bytes",
        bytes.len()
    );

    // Random → NO false generator: falls back to the bit-pack (~125 B) and round-trips.
    let mut rng = SplitMix64 {
        state: 0xB001_C0DE_F00D,
    };
    let random: Vec<bool> = (0..1000).map(|_| rng.next() & 1 == 0).collect();
    let (bytes, got) = roundtrip(&random);
    assert_eq!(
        got, random,
        "random bools round-trip (no false generator detection)"
    );
    assert!(
        bytes.len() >= 100,
        "random bools fall back to the bit-pack, not a false generator: {} bytes",
        bytes.len()
    );

    // Edges: empty + single element round-trip cleanly.
    for edge in [
        vec![],
        vec![true],
        vec![false],
        vec![true, false],
        vec![false, false, false],
    ] {
        let (_, got) = roundtrip(&edge);
        assert_eq!(got, edge, "bool edge case {edge:?} round-trips");
    }
}

#[test]
fn wire_string_affix_ships_common_prefix_and_suffix_with_arbitrary_middles() {
    // The last string-column gap: a column sharing a common PREFIX and/or SUFFIX with ARBITRARY
    // middles — what the dictionary (all distinct), the template (non-affine middles), and front-
    // coding (shares only a PREFIX, pairwise) all miss.
    fn roundtrip(strings: &[String]) -> (Vec<u8>, Vec<String>) {
        let items: Vec<RuntimeValue> = strings
            .iter()
            .map(|s| RuntimeValue::Text(Rc::new(s.clone())))
            .collect();
        let val = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(items))));
        let bytes = with_structure(WireStructure::Auto, || {
            message_to_wire_with("", &val, WireCodec::Native, WireIntegrity::Raw).unwrap()
        });
        let got = match message_from_wire(&bytes).unwrap().1 {
            RuntimeValue::List(l) => {
                let b = l.borrow();
                (0..b.len())
                    .map(|i| match b.get(i).unwrap() {
                        RuntimeValue::Text(s) => (*s).clone(),
                        other => panic!("expected Text, got {other:?}"),
                    })
                    .collect()
            }
            _ => panic!("expected List"),
        };
        (bytes, got)
    }
    let flat_len = |v: &[String]| -> usize { v.iter().map(|s| s.len()).sum() };

    // Emails — common SUFFIX `@example.com`, arbitrary names (front-coding shares only prefixes,
    // so it can't crush the repeated suffix; affix ships the suffix ONCE).
    let names = [
        "alice", "bob", "charlie", "dave", "erin", "frank", "grace", "heidi",
    ];
    let emails: Vec<String> = (0..400)
        .map(|i| format!("{}{}@example.com", names[i % names.len()], i))
        .collect();
    let (bytes, got) = roundtrip(&emails);
    assert_eq!(got, emails, "affixed emails reconstruct exactly");
    assert!(
        bytes.len() * 2 < flat_len(&emails),
        "affix ships the shared @example.com suffix once: {} vs flat {}",
        bytes.len(),
        flat_len(&emails)
    );

    // Common extension `.log` (suffix only, arbitrary stems that don't sort-share prefixes).
    let stems = ["app", "db", "auth", "cache", "queue", "web", "cron", "mail"];
    let files: Vec<String> = (0..400)
        .map(|i| format!("{}-{}.log", stems[i % stems.len()], i * 7))
        .collect();
    let (bytes, got) = roundtrip(&files);
    assert_eq!(got, files, "affixed log files reconstruct exactly");
    assert!(
        bytes.len() < flat_len(&files),
        "affix beats flat on a shared extension"
    );

    // Both prefix AND suffix, arbitrary (non-affine) middle → wrapped versioned ids.
    let mut rng = SplitMix64 { state: 0xAFF1_C0DE };
    let wrapped: Vec<String> = (0..300)
        .map(|_| {
            format!(
                "https://cdn.example.com/v2/{}/asset.json",
                rng.next() % 1_000_000
            )
        })
        .collect();
    let (bytes, got) = roundtrip(&wrapped);
    assert_eq!(
        got, wrapped,
        "prefix+suffix wrapped ids reconstruct exactly"
    );
    assert!(
        bytes.len() * 2 < flat_len(&wrapped),
        "affix crushes the shared URL wrapper"
    );

    // NO false win: a column with NO shared affix round-trips via flat/dict (affix returns None
    // when prefix+suffix is empty).
    let mut rng2 = SplitMix64 { state: 0x0FF1_DEAD };
    let scattered: Vec<String> = (0..200)
        .map(|_| {
            let len = 5 + (rng2.next() % 8) as usize;
            (0..len)
                .map(|_| (b'a' + (rng2.next() % 26) as u8) as char)
                .collect()
        })
        .collect();
    let (_, got) = roundtrip(&scattered);
    assert_eq!(
        got, scattered,
        "affix-free random strings round-trip (no false affix)"
    );
}

#[test]
fn wire_gen_expr_substrate_round_trips_and_evaluates() {
    // `(i % 2 == 0) ? i*10 : i*10 + 5` — a piecewise column. The sandbox evaluates it
    // bit-exact, the tree round-trips through serialize/deserialize, and a T_GEN value
    // decodes to the evaluated column. This is the substrate a pure user function lowers
    // into — the receiver runs only this bounded evaluator, never arbitrary code.
    let expr = GenExpr::Select {
        op: GenCmp::Eq,
        lhs: Box::new(GenExpr::Mod(
            Box::new(GenExpr::Index),
            Box::new(GenExpr::Const(2)),
        )),
        rhs: Box::new(GenExpr::Const(0)),
        then: Box::new(GenExpr::Mul(
            Box::new(GenExpr::Index),
            Box::new(GenExpr::Const(10)),
        )),
        els: Box::new(GenExpr::Add(
            Box::new(GenExpr::Mul(
                Box::new(GenExpr::Index),
                Box::new(GenExpr::Const(10)),
            )),
            Box::new(GenExpr::Const(5)),
        )),
    };
    let expected: Vec<i64> = (0..256i64)
        .map(|i| if i % 2 == 0 { i * 10 } else { i * 10 + 5 })
        .collect();
    for (i, &e) in expected.iter().enumerate() {
        assert_eq!(gen_eval(&expr, i as i64), e, "sandbox eval matches at {i}");
    }
    let mut sbytes = Vec::new();
    serialize_gen(&expr, &mut sbytes);
    let mut p = 0;
    let mut budget = MAX_GEN_NODES;
    assert_eq!(
        deserialize_gen(&sbytes, &mut p, &mut budget, 0),
        Some(expr.clone()),
        "GenExpr round-trips"
    );
    assert_eq!(
        p,
        sbytes.len(),
        "the tree is self-delimiting (no trailing bytes)"
    );

    let mut bytes = vec![T_GEN];
    serialize_gen(&expr, &mut bytes);
    write_uvarint(expected.len() as u64, &mut bytes);
    let mut p = 0;
    match native_decode(&bytes, &mut p).expect("T_GEN decodes") {
        RuntimeValue::List(l) => match &*l.borrow() {
            ListRepr::Ints(got) => assert_eq!(got, &expected, "T_GEN evaluates to the column"),
            _ => panic!("expected Ints"),
        },
        _ => panic!("expected List"),
    }
}

#[test]
fn wire_lower_pure_function_to_genexpr() {
    // The bridge for "ship a user function": a pure single-param arithmetic function
    // lowers to the sandboxed GenExpr (so the receiver evaluates data, never runs code).
    // `f(i) = 3*i*i - 5*i + 7` lowers and evaluates identically; anything outside the
    // safe arithmetic subset (unknown var, a call) is refused — never shippable.
    use crate::ast::stmt::{BinaryOpKind, Expr, Literal};
    use logicaffeine_base::{Arena, Symbol};
    fn num<'a>(a: &'a Arena<Expr<'a>>, v: i64) -> &'a Expr<'a> {
        a.alloc(Expr::Literal(Literal::Number(v)))
    }
    fn bin<'a>(
        a: &'a Arena<Expr<'a>>,
        op: BinaryOpKind,
        l: &'a Expr<'a>,
        r: &'a Expr<'a>,
    ) -> &'a Expr<'a> {
        a.alloc(Expr::BinaryOp {
            op,
            left: l,
            right: r,
        })
    }
    let a: Arena<Expr> = Arena::new();
    let i = Symbol::from_index(0);
    let idx: &Expr = a.alloc(Expr::Identifier(i));
    // 3*i*i - 5*i + 7
    let body = bin(
        &a,
        BinaryOpKind::Add,
        bin(
            &a,
            BinaryOpKind::Subtract,
            bin(
                &a,
                BinaryOpKind::Multiply,
                bin(&a, BinaryOpKind::Multiply, num(&a, 3), idx),
                idx,
            ),
            bin(&a, BinaryOpKind::Multiply, num(&a, 5), idx),
        ),
        num(&a, 7),
    );
    let g = lower_expr_to_genexpr(body, i).expect("pure arithmetic lowers");
    for x in -50..50i64 {
        assert_eq!(
            gen_eval(&g, x),
            3 * x * x - 5 * x + 7,
            "lowered generator matches f at {x}"
        );
    }
    let other: &Expr = a.alloc(Expr::Identifier(Symbol::from_index(1)));
    assert!(
        lower_expr_to_genexpr(other, i).is_none(),
        "unknown variable → not shippable"
    );
    let call: &Expr = a.alloc(Expr::Call {
        function: Symbol::from_index(2),
        args: vec![],
    });
    assert!(
        lower_expr_to_genexpr(call, i).is_none(),
        "a function call → not shippable"
    );
}

#[test]
fn wire_computed_function_ships_callable_and_round_trips() {
    // A pure single-arg function lowered to a generator ships as T_FUNC, round-trips, and
    // the decoded function carries a generator that evaluates f(x) — a CALLABLE on a peer
    // that never compiled it. An ordinary closure (no generator) stays unsendable.
    use crate::ast::stmt::{BinaryOpKind, Expr, Literal};
    use logicaffeine_base::{Arena, Symbol};
    let a: Arena<Expr> = Arena::new();
    let i = Symbol::from_index(0);
    let idx: &Expr = a.alloc(Expr::Identifier(i));
    let sq: &Expr = a.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Multiply,
        left: idx,
        right: idx,
    });
    let one: &Expr = a.alloc(Expr::Literal(Literal::Number(1)));
    let body: &Expr = a.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left: sq,
        right: one,
    });
    let expr = lower_expr_to_genexpr(body, i).expect("f(i)=i*i+1 lowers");

    let f = RuntimeValue::Function(Box::new(ClosureValue {
        body_index: usize::MAX,
        captured_env: std::collections::HashMap::default(),
        param_names: vec![i],
        generated: Some(Rc::new(expr.clone())),
    }));
    let bytes = message_to_wire("p", &f).expect("a generated function is sendable");
    match message_from_wire(&bytes).unwrap().1 {
        RuntimeValue::Function(c) => {
            let g = c.generated.expect("decoded function carries its generator");
            assert_eq!(&*g, &expr, "the generator round-trips exactly");
            assert_eq!(c.param_names.len(), 1, "arity preserved");
            for x in -20..20i64 {
                assert_eq!(
                    gen_eval(&g, x),
                    x * x + 1,
                    "the shipped function evaluates f(x) on the receiver"
                );
            }
        }
        _ => panic!("expected a Function back"),
    }

    let plain = RuntimeValue::Function(Box::new(ClosureValue {
        body_index: 0,
        captured_env: std::collections::HashMap::default(),
        param_names: vec![],
        generated: None,
    }));
    assert!(
        message_to_wire("p", &plain).is_err(),
        "an un-lowered closure is still not sendable"
    );
}

#[test]
fn receiver_refuses_a_shipped_computation_when_it_declines_code() {
    // THE EXECUTABLE-PAYLOAD GATE. The ONLY "code" the wire can carry is a bounded GenExpr sandbox
    // (never arbitrary bytecode — un-lowered closures are unsendable, proven above). A receiver that
    // declares `accept_computed: false` REFUSES even that, at decode — the first of three gates
    // against running code you didn't ask for (the C2 acceptance contract gates INVOCATION; the
    // sandbox bounds what an accepted call can do).
    use crate::ast::stmt::{BinaryOpKind, Expr, Literal};
    use logicaffeine_base::{Arena, Symbol};
    let a: Arena<Expr> = Arena::new();
    let i = Symbol::from_index(0);
    let idx: &Expr = a.alloc(Expr::Identifier(i));
    let one: &Expr = a.alloc(Expr::Literal(Literal::Number(1)));
    let body: &Expr = a.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left: idx,
        right: one,
    });
    let expr = lower_expr_to_genexpr(body, i).expect("f(i)=i+1 lowers");
    let f = RuntimeValue::Function(Box::new(ClosureValue {
        body_index: usize::MAX,
        captured_env: std::collections::HashMap::default(),
        param_names: vec![i],
        generated: Some(Rc::new(expr)),
    }));
    let bytes = message_to_wire("p", &f).expect("a generated function is sendable");
    // Default: the computation decodes (it stays INERT until invoked through an acceptance contract).
    assert!(
        message_from_wire(&bytes).is_some(),
        "by default a shipped computation decodes (inert data until a contract invokes it)"
    );
    // A receiver that declines code refuses to decode it at all.
    let no_code = ReceiveLimits {
        accept_computed: false,
        ..Default::default()
    };
    assert!(
        with_receive_limits(no_code, || message_from_wire(&bytes)).is_none(),
        "a receiver with accept_computed=false must REFUSE a shipped computation at decode"
    );
}

#[test]
fn wire_auto_modular_affine_ships_a_generator() {
    // A sawtooth column `a + b·(i mod p)` ships a tiny GenExpr, not the values, bit-exact.
    let v: Vec<i64> = (0..5000i64).map(|i| 1000 + 7 * (i % 12)).collect();
    let (auto, got) = affine_roundtrip(&v, WireStructure::Auto);
    assert_eq!(got, v, "the modular-affine column reconstructs bit-exact");
    let (varint, _) = affine_roundtrip(&v, WireStructure::Off);
    assert!(
        auto.len() < varint.len() / 50,
        "the generator ({} B) must ship ≪ the data ({} B)",
        auto.len(),
        varint.len()
    );
}

#[test]
fn wire_auto_byte_column_ships_a_tight_zero_copy_blob() {
    // Wide-range, STRUCTURE-FREE bytes (deterministic random, no affine/geometric/periodic
    // law to exploit) ship as a tight 1-byte-per-element blob AND read back in place as
    // `&[u8]` with zero copy — the first-class binary type (capnp `Data`, protobuf `bytes`)
    // that FOR's bit-packing can't offer. Beats varint (2 bytes on every ≥128). The data is
    // random precisely so the GENERATORS (which legitimately win on structured columns) don't
    // preempt the blob — arbitrary binary is exactly when the zero-copy blob is the best form.
    let mut rng = SplitMix64 {
        state: 0x6B70_B10B_CAFE_F00D,
    };
    let data: Vec<i64> = (0..1000).map(|_| (rng.next() % 256) as i64).collect();
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(data.clone()))));
    let bytes = with_structure(WireStructure::Auto, || message_to_wire("p", &v).unwrap());

    match &message_from_wire(&bytes).unwrap().1 {
        RuntimeValue::List(l) => match &*l.borrow() {
            ListRepr::Ints(g) => assert_eq!(g, &data, "byte column round-trips bit-exact"),
            _ => panic!("expected Ints"),
        },
        _ => panic!("expected List"),
    }
    assert!(
        bytes.len() <= data.len() + 32,
        "tight blob: {} bytes for {} values",
        bytes.len(),
        data.len()
    );

    let view = view_message(&bytes).unwrap();
    let slice = view
        .as_byte_slice()
        .expect("a byte column reads zero-copy as &[u8]");
    assert_eq!(slice.len(), data.len(), "all bytes present");
    for (i, &b) in slice.iter().enumerate() {
        assert_eq!(b as i64, data[i], "byte {i} matches");
    }
    let base = bytes.as_ptr() as usize;
    let lo = slice.as_ptr() as usize;
    assert!(
        lo >= base && lo < base + bytes.len(),
        "the &[u8] borrows the message buffer (zero-copy)"
    );

    let varint = with_structure(WireStructure::Off, || message_to_wire("p", &v).unwrap());
    assert!(
        bytes.len() < varint.len(),
        "byte blob ({}) beats varint ({})",
        bytes.len(),
        varint.len()
    );
}

#[test]
fn wire_narrow_byte_column_prefers_for_when_smaller() {
    // Narrow-range bytes (0..16) bit-pack SMALLER via FOR (≈4 bits each) than a 1-byte
    // blob, so the menu correctly chooses FOR — `T_BYTES` is only selected when it is
    // genuinely the best, never as a redundant default.
    let data: Vec<i64> = (0..1000).map(|i: i64| (i * 7).rem_euclid(16)).collect();
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(data.clone()))));
    let bytes = with_structure(WireStructure::Auto, || message_to_wire("p", &v).unwrap());

    match &message_from_wire(&bytes).unwrap().1 {
        RuntimeValue::List(l) => match &*l.borrow() {
            ListRepr::Ints(g) => assert_eq!(g, &data, "narrow byte column round-trips"),
            _ => panic!("expected Ints"),
        },
        _ => panic!("expected List"),
    }
    assert!(
        bytes.len() < data.len(),
        "narrow bytes pack below 1 byte/element ({} for {})",
        bytes.len(),
        data.len()
    );
    assert!(
        view_message(&bytes).unwrap().as_byte_slice().is_none(),
        "the narrow column uses FOR/dict, not a T_BYTES blob"
    );
}

#[test]
fn wire_gen_decode_rejects_hostile_trees() {
    // Sandbox safety: an over-deep tree is rejected (not stack-overflowed); garbage and
    // truncated trees return None; a T_GEN value with a hostile body decodes to None —
    // never a panic, never unbounded work.
    let mut deep = GenExpr::Index;
    for _ in 0..(MAX_GEN_DEPTH + 5) {
        deep = GenExpr::Add(Box::new(deep), Box::new(GenExpr::Const(1)));
    }
    let mut sbytes = Vec::new();
    serialize_gen(&deep, &mut sbytes);
    let mut p = 0;
    let mut budget = MAX_GEN_NODES;
    assert!(
        deserialize_gen(&sbytes, &mut p, &mut budget, 0).is_none(),
        "over-deep tree rejected"
    );

    let mut p = 0;
    let mut budget = MAX_GEN_NODES;
    assert!(
        deserialize_gen(&[99u8], &mut p, &mut budget, 0).is_none(),
        "garbage node tag → None"
    );

    let mut p = 0;
    let mut budget = MAX_GEN_NODES;
    assert!(
        deserialize_gen(&[2u8], &mut p, &mut budget, 0).is_none(),
        "truncated binary op → None"
    );

    let mut p = 0;
    assert!(
        native_decode(&[T_GEN, 99u8, 5u8], &mut p).is_none(),
        "garbage T_GEN body → None, no panic"
    );
}

#[test]
fn a_provisional_answer_holding_another_is_refused_on_the_wire() {
    let inner = crate::semantics::exact::provisional(RuntimeValue::Int(3), 10, 1);
    let mut nested = vec![T_CONVERGING];
    native_encode(&inner, &mut nested).unwrap();
    write_uvarint(5, &mut nested);
    write_uvarint(0, &mut nested);
    nested.push(0);
    let mut p = 0;
    assert!(native_decode(&nested, &mut p).is_none(), "an answer never holds another");

    let joined = crate::semantics::exact::provisional(inner, 5, 0);
    let mut bytes = Vec::new();
    native_encode(&joined, &mut bytes).unwrap();
    let mut p = 0;
    let back = native_decode(&bytes, &mut p).expect("the joined answer decodes");
    assert_eq!(p, bytes.len());
    assert!(crate::semantics::compare::values_equal(&back, &joined));
    assert_eq!(
        crate::semantics::exact::display(&back).as_deref(),
        Some("3 (provisional: checked 5 cases, 1 revision; not proved)")
    );
}

#[test]
fn wire_matrix_blast_every_knob_combo_composes() {
    // MATRIX BLAST: the full Cartesian product of every codec knob, over representative
    // payloads. The risk in chaining knobs is INTERFERENCE — one silently corrupting
    // another. For each (payload × numerics × structure × floats × compression × integrity
    // × struct-view) we (1) encode, (2) decode without panic, and (3) prove the decoded
    // value is canonically IDENTICAL to the original by re-normalizing both to one fixed
    // baseline encoding and comparing bytes. So every combination must round-trip exactly.
    // Adding a knob = adding one loop dimension here; nothing else can quietly break.
    fn li(v: Vec<i64>) -> RuntimeValue {
        RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(v))))
    }
    fn lf(v: Vec<f64>) -> RuntimeValue {
        RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(v))))
    }
    fn rec(id: i64, name: &str, active: bool) -> RuntimeValue {
        let mut f = HashMap::new();
        f.insert("id".to_string(), RuntimeValue::Int(id));
        f.insert(
            "name".to_string(),
            RuntimeValue::Text(Rc::new(name.to_string())),
        );
        f.insert("active".to_string(), RuntimeValue::Bool(active));
        RuntimeValue::Struct(Box::new(StructValue {
            type_name: "Rec".to_string(),
            fields: f,
        }))
    }
    let names = ["a", "bb", "ccc", "dddd"];
    let payloads: Vec<(&str, RuntimeValue)> = vec![
        ("scalar int", RuntimeValue::Int(-123456789)),
        ("bigint", RuntimeValue::Int(i64::MIN)),
        (
            "text",
            RuntimeValue::Text(Rc::new("hello, wire".to_string())),
        ),
        ("bool", RuntimeValue::Bool(true)),
        ("nothing", RuntimeValue::Nothing),
        (
            "random ints",
            li((0..64)
                .map(|i: i64| i.wrapping_mul(2_654_435_761))
                .collect()),
        ),
        (
            "monotone ints",
            li((0..64i64)
                .scan(1000i64, |s, i| {
                    *s += 1 + (i % 3);
                    Some(*s)
                })
                .collect()),
        ),
        (
            "poly ints",
            li((0..64).map(|i: i64| 3 * i * i - 5 * i + 7).collect()),
        ),
        (
            "sawtooth ints",
            li((0..64).map(|i: i64| 100 + 5 * (i % 9)).collect()),
        ),
        ("clustered ints", li((0..64).map(|_| 1_000_000).collect())),
        (
            "wide bytes",
            li((0..64)
                .map(|i: i64| (i * 37 + 128).rem_euclid(256))
                .collect()),
        ),
        (
            "narrow bytes",
            li((0..64).map(|i: i64| (i * 7).rem_euclid(16)).collect()),
        ),
        (
            "floats",
            lf((0..64).map(|i| i as f64 * 1.5 - 7.0).collect()),
        ),
        (
            "special floats",
            lf(vec![
                f64::NAN,
                f64::INFINITY,
                f64::NEG_INFINITY,
                -0.0,
                f64::MIN_POSITIVE,
            ]),
        ),
        (
            "bools",
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(
                (0..64).map(|i| RuntimeValue::Bool(i % 3 == 0)).collect(),
            )))),
        ),
        (
            "strings",
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(
                (0..64)
                    .map(|i| RuntimeValue::Text(Rc::new(names[i % 4].to_string())))
                    .collect(),
            )))),
        ),
        ("struct", rec(7, "lone", false)),
        (
            "struct list",
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(
                (0..16)
                    .map(|i| rec(i, names[i as usize % 4], i % 2 == 0))
                    .collect(),
            )))),
        ),
        ("empty list", li(vec![])),
        ("int→int map", {
            let mut m = MapStorage::default();
            for k in 0..32i64 {
                m.insert(RuntimeValue::Int(k), RuntimeValue::Int(k * k));
            }
            RuntimeValue::Map(Rc::new(RefCell::new(m)))
        }),
        ("int→string map", {
            let mut m = MapStorage::default();
            for k in 0..16i64 {
                m.insert(
                    RuntimeValue::Int(k * 10),
                    RuntimeValue::Text(Rc::new(format!("row_{k}"))),
                );
            }
            RuntimeValue::Map(Rc::new(RefCell::new(m)))
        }),
        ("int→struct map", {
            // kind-0 values respond to the dials (struct_view etc.) — composes the map crush with
            // the struct encoders across every combo.
            let mut m = MapStorage::default();
            for k in 0..8i64 {
                m.insert(
                    RuntimeValue::Int(k),
                    rec(k, names[k as usize % 4], k % 2 == 0),
                );
            }
            RuntimeValue::Map(Rc::new(RefCell::new(m)))
        }),
        ("text→int map", {
            let mut m = MapStorage::default();
            for k in 0..8i64 {
                m.insert(
                    RuntimeValue::Text(Rc::new(format!("k{k}"))),
                    RuntimeValue::Int(k),
                );
            }
            RuntimeValue::Map(Rc::new(RefCell::new(m)))
        }),
    ];

    // The fixed normalizer: encode under one canonical knob set so two values that are
    // canonically equal produce identical bytes regardless of how they were transmitted.
    let canon = |val: &RuntimeValue| -> Vec<u8> {
        with_numerics(WireNumerics::Varint, || {
            with_structure(WireStructure::Off, || {
                with_floats(WireFloats::Memcpy, || {
                    message_to_wire_with("c", val, WireCodec::Native, WireIntegrity::Raw).unwrap()
                })
            })
        })
    };

    let mut combos = 0u64;
    for (name, v) in &payloads {
        let want = canon(v);
        for num in [
            WireNumerics::Varint,
            WireNumerics::Fixed,
            WireNumerics::GroupVarint,
        ] {
            for st in [
                WireStructure::Off,
                WireStructure::Affine,
                WireStructure::Auto,
            ] {
                for fl in [WireFloats::Memcpy, WireFloats::XorDelta] {
                    for comp in [
                        WireCompression::None,
                        WireCompression::Deflate,
                        WireCompression::Lz4,
                        WireCompression::Zstd,
                    ] {
                        for integ in [WireIntegrity::Raw, WireIntegrity::Checked] {
                            for sv in [false, true] {
                                let bytes = with_numerics(num, || {
                                    with_structure(st, || {
                                        with_floats(fl, || {
                                            with_compression_codec(comp, || {
                                                with_struct_view(sv, || {
                                                    message_to_wire_with(
                                                        "p",
                                                        v,
                                                        WireCodec::Native,
                                                        integ,
                                                    )
                                                    .unwrap()
                                                })
                                            })
                                        })
                                    })
                                });
                                let back = message_from_wire(&bytes).unwrap_or_else(|| {
                                    panic!(
                                        "{name} failed to decode under num={num:?} st={st:?} fl={fl:?} \
                                             comp={comp:?} integ={integ:?} sv={sv}"
                                    )
                                });
                                assert_eq!(
                                    canon(&back.1),
                                    want,
                                    "{name} corrupted under num={num:?} st={st:?} fl={fl:?} \
                                         comp={comp:?} integ={integ:?} sv={sv}"
                                );
                                combos += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(
        combos >= 4000,
        "matrix should blast thousands of knob combos, ran {combos}"
    );
}

#[test]
fn wire_shared_type_id_composes_with_every_knob() {
    // Type-id elision (`shared`) drops struct names to a small id. This focuses on STRUCT
    // payloads — the only shapes type-id touches — and verifies the elided form COMPOSES
    // with every other knob: encode + decode under a registry, across all dials, must
    // decode canonically identical to the self-describing form. Kept small (structs only)
    // so it stays quick while still chaining the registry knob against everything else.
    fn rec(id: i64, name: &str, active: bool) -> RuntimeValue {
        let mut f = HashMap::new();
        f.insert("id".to_string(), RuntimeValue::Int(id));
        f.insert(
            "name".to_string(),
            RuntimeValue::Text(Rc::new(name.to_string())),
        );
        f.insert("active".to_string(), RuntimeValue::Bool(active));
        RuntimeValue::Struct(Box::new(StructValue {
            type_name: "Rec".to_string(),
            fields: f,
        }))
    }
    let names = ["a", "bb", "ccc"];
    let payloads = vec![
        rec(7, "lone", false),
        RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(
            (0..12)
                .map(|i| rec(i, names[i as usize % 3], i % 2 == 0))
                .collect(),
        )))),
    ];
    let mk_reg = || {
        WireTypeRegistry::new(vec![(
            "Rec".to_string(),
            vec!["active".to_string(), "id".to_string(), "name".to_string()],
        )])
    };
    let canon = |val: &RuntimeValue| -> Vec<u8> {
        with_numerics(WireNumerics::Varint, || {
            with_structure(WireStructure::Off, || {
                message_to_wire_with("c", val, WireCodec::Native, WireIntegrity::Raw).unwrap()
            })
        })
    };
    let mut combos = 0u32;
    for v in &payloads {
        let want = canon(v);
        for num in [
            WireNumerics::Varint,
            WireNumerics::Fixed,
            WireNumerics::GroupVarint,
        ] {
            for st in [
                WireStructure::Off,
                WireStructure::Affine,
                WireStructure::Auto,
            ] {
                for comp in [WireCompression::None, WireCompression::Zstd] {
                    for integ in [WireIntegrity::Raw, WireIntegrity::Checked] {
                        for sv in [false, true] {
                            let enc = || {
                                with_numerics(num, || {
                                    with_structure(st, || {
                                        with_compression_codec(comp, || {
                                            with_struct_view(sv, || {
                                                message_to_wire_with(
                                                    "p",
                                                    v,
                                                    WireCodec::Native,
                                                    integ,
                                                )
                                                .unwrap()
                                            })
                                        })
                                    })
                                })
                            };
                            // Type-id active for BOTH encode and decode (the id must resolve).
                            let bytes = with_type_registry(mk_reg(), enc);
                            let back = with_type_registry(mk_reg(), || message_from_wire(&bytes))
                                .expect("a type-id-elided struct decodes with the registry");
                            assert_eq!(
                                canon(&back.1),
                                want,
                                "shared struct corrupted under num={num:?} st={st:?} comp={comp:?} integ={integ:?} sv={sv}"
                            );
                            combos += 1;
                        }
                    }
                }
            }
        }
    }
    assert!(combos >= 100, "shared-knob matrix ran {combos} combos");
}

#[test]
fn wire_auto_rle_wins_on_runs() {
    // 20 runs of 30 identical values → run-length collapses each run to one pair.
    let mut v = Vec::new();
    for k in 0..20i64 {
        for _ in 0..30 {
            v.push(k);
        }
    }
    let (auto, got) = affine_roundtrip(&v, WireStructure::Auto);
    let (varint, _) = affine_roundtrip(&v, WireStructure::Off);
    assert_eq!(got, v, "run-length round-trips bit-exact");
    assert!(
        auto.len() < varint.len(),
        "Auto ({}) must beat varint ({}) on runs",
        auto.len(),
        varint.len()
    );
}

#[test]
fn wire_auto_dict_wins_on_low_cardinality() {
    // Five scattered distinct values (no runs, so RLE can't win) → dictionary + narrow
    // bit-packed indices.
    let mut rng = SplitMix64 { state: 0x0000_BEEF };
    let palette = [7i64, 42, 1000, -5, 999_999];
    let v: Vec<i64> = (0..500)
        .map(|_| palette[(rng.next() % 5) as usize])
        .collect();
    let (auto, got) = affine_roundtrip(&v, WireStructure::Auto);
    let (varint, _) = affine_roundtrip(&v, WireStructure::Off);
    assert_eq!(got, v, "dictionary round-trips bit-exact");
    assert!(
        auto.len() < varint.len(),
        "Auto ({}) must beat varint ({}) on low cardinality",
        auto.len(),
        varint.len()
    );
}

#[test]
fn wire_auto_never_worse_than_varint_on_random() {
    // Full-range random columns have no structure: the selector always includes the
    // plain-varint baseline, so Auto is never larger — and it still round-trips exactly.
    let mut rng = SplitMix64 { state: 0x00C0_FFEE };
    for _ in 0..50 {
        let n = (rng.next() % 200) as usize;
        let v: Vec<i64> = (0..n).map(|_| rng.next() as i64).collect();
        let (auto, got) = affine_roundtrip(&v, WireStructure::Auto);
        let (varint, _) = affine_roundtrip(&v, WireStructure::Off);
        assert_eq!(got, v, "Auto round-trips bit-exact on random data");
        assert!(
            auto.len() <= varint.len(),
            "Auto ({}) must never exceed varint ({})",
            auto.len(),
            varint.len()
        );
    }
}

#[test]
fn wire_auto_decoder_never_panics_on_mutated_menu_messages() {
    // Every compression-menu tag, byte-mutated, must fail cleanly (None) — never panic
    // or over-allocate (the RLE/bit-pack lengths are attacker-controlled here).
    let mut rng = SplitMix64 { state: 0x00AB_1234 };
    let shapes: Vec<Vec<i64>> = vec![
        (0..50i64).collect(),
        vec![5i64; 80],
        (0..60i64).map(|i| 1_000_000 + (i % 8)).collect(),
        (0..70i64).map(|i| 1_700_000_000 + i * 7).collect(),
    ];
    for shape in &shapes {
        let value = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(shape.clone()))));
        let base = with_structure(WireStructure::Auto, || {
            message_to_wire_with("", &value, WireCodec::Native, WireIntegrity::Raw).unwrap()
        });
        for _ in 0..2000 {
            let mut m = base.clone();
            let i = (rng.next() as usize) % m.len();
            m[i] ^= (rng.next() & 0xFF) as u8;
            let _ = message_from_wire(&m); // must not panic
        }
    }
}

#[test]
fn wire_intkey_map_decoder_never_panics_on_mutation() {
    // The T_MAP_INTKEY two-column form (int key column + value-kind byte + value column / per-value
    // list) is fully attacker-controlled on decode: the kind byte, both column lengths, and the
    // kind-0 value count can each be flipped. Byte-mutating both an int→int (kind 1) and an
    // int→text (kind 0) map message must fail cleanly — never panic, never over-allocate.
    let mut rng = SplitMix64 { state: 0x00CD_5678 };
    let mk_int = {
        let mut m = MapStorage::default();
        for k in 0..40i64 {
            m.insert(RuntimeValue::Int(k), RuntimeValue::Int(k * 3));
        }
        RuntimeValue::Map(Rc::new(RefCell::new(m)))
    };
    let mk_text = {
        let mut m = MapStorage::default();
        for k in 0..12i64 {
            m.insert(
                RuntimeValue::Int(k * 100),
                RuntimeValue::Text(Rc::new(format!("v{k}"))),
            );
        }
        RuntimeValue::Map(Rc::new(RefCell::new(m)))
    };
    let mk_struct = {
        let mut m = MapStorage::default();
        for k in 0..10i64 {
            let mut f = HashMap::new();
            f.insert("id".to_string(), RuntimeValue::Int(k));
            f.insert("ok".to_string(), RuntimeValue::Bool(k % 2 == 0));
            m.insert(
                RuntimeValue::Int(k),
                RuntimeValue::Struct(Box::new(StructValue {
                    type_name: "R".to_string(),
                    fields: f,
                })),
            );
        }
        RuntimeValue::Map(Rc::new(RefCell::new(m)))
    };
    for value in [mk_int, mk_text, mk_struct] {
        let base = message_to_wire_with("", &value, WireCodec::Native, WireIntegrity::Raw).unwrap();
        for _ in 0..3000 {
            let mut b = base.clone();
            let i = (rng.next() as usize) % b.len();
            b[i] ^= (rng.next() & 0xFF) as u8;
            let _ = message_from_wire(&b); // must not panic / OOM
        }
    }
}

/// Encode an `Ints` list through the LEB128 varint path (structure off, the default
/// `Varint` numerics), decode it, and return (bytes, recovered) — the oracle for the
/// adaptive sign-mode encoding.
fn varint_roundtrip(v: &[i64]) -> (Vec<u8>, Vec<i64>) {
    let value = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(v.to_vec()))));
    let bytes = with_structure(WireStructure::Off, || {
        message_to_wire_with("", &value, WireCodec::Native, WireIntegrity::Raw).unwrap()
    });
    let (_, back) = message_from_wire(&bytes).expect("decode");
    let got = match back {
        RuntimeValue::List(l) => match &*l.borrow() {
            ListRepr::Ints(g) => g.clone(),
            _ => panic!("expected an Ints list back"),
        },
        _ => panic!("expected a List back"),
    };
    (bytes, got)
}

#[test]
fn a_non_negative_int_column_skips_zigzag_for_half_the_size() {
    // 100 copies of 64: plain LEB128 is one byte each (64 < 128); zigzag would spend
    // two (zigzag(64) = 128). The non-negative column must skip zigzag — matching
    // protobuf's `int64` instead of paying its `sint64` doubling penalty.
    let v = vec![64i64; 100];
    let (bytes, got) = varint_roundtrip(&v);
    assert_eq!(got, v, "non-negative round-trip must be exact");
    assert!(
        bytes.len() < 150,
        "a non-negative column must use plain LEB128 (≈100B), not zigzag (≈200B); got {} bytes",
        bytes.len()
    );
}

#[test]
fn a_column_with_any_negative_uses_zigzag_and_round_trips() {
    let v = vec![-1i64, -64, 5, -100, 0, 127];
    let (_, got) = varint_roundtrip(&v);
    assert_eq!(got, v, "mixed-sign round-trip must be exact");
}

#[test]
fn adaptive_sign_mode_round_trips_random_columns() {
    // Fill with RANDOM data (a deterministic xorshift, so the test is reproducible)
    // to be honest about the sign-mode decision: ~half the columns are forced
    // all-non-negative (exercising plain LEB128), the rest are full-range i64
    // (exercising zig-zag). Every column must round-trip exactly.
    let mut rng = 0x1234_5678_9ABC_DEF0u64;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    for _ in 0..3000 {
        let n = (next() % 80) as usize;
        let force_nonneg = next() & 1 == 0;
        let v: Vec<i64> = (0..n)
            .map(|_| {
                let r = next() as i64;
                if force_nonneg {
                    r & i64::MAX // clear the sign bit → non-negative
                } else {
                    r
                }
            })
            .collect();
        let (_, got) = varint_roundtrip(&v);
        assert_eq!(
            got, v,
            "random adaptive sign-mode round-trip failed for {v:?}"
        );
    }
}

#[test]
fn adaptive_sign_mode_round_trips_every_boundary() {
    let cases: Vec<Vec<i64>> = vec![
        vec![],
        vec![0i64; 10],
        vec![1, 2, 3, 63, 64, 65, 127, 128, 255, 256],
        vec![-1, -2, -63, -64, -128, -129],
        vec![i64::MIN, i64::MAX, 0, -1, 1],
        vec![i64::MAX, i64::MAX - 1], // all non-negative, max magnitude
    ];
    for v in cases {
        let (_, got) = varint_roundtrip(&v);
        assert_eq!(got, v, "adaptive sign-mode round-trip failed for {v:?}");
    }
}

#[test]
fn wireview_reads_any_element_in_place_without_decoding() {
    // A million-element fixed-width int column. The view reads element N directly at
    // its byte offset — no full decode, no allocation, any index.
    let v: Vec<i64> = (0..1_000_000).map(|i| i as i64 * 3 - 7).collect();
    let value = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(v.clone()))));
    let bytes = with_numerics(WireNumerics::Fixed, || {
        message_to_wire_with("", &value, WireCodec::Native, WireIntegrity::Raw).unwrap()
    });
    let view = view_message(&bytes).expect("view opens over the fixed-layout message");
    assert_eq!(view.int_list_len(), Some(1_000_000));
    for &i in &[0usize, 1, 12_345, 678_901, 999_999] {
        assert_eq!(
            view.int_list_get(i),
            Some(v[i]),
            "random-access element {i}"
        );
    }
    assert_eq!(
        view.int_list_get(1_000_000),
        None,
        "out-of-bounds is None, not a panic"
    );
}

#[test]
fn wireview_varint_and_float_and_scalar_views_agree_with_decode() {
    // The view must read varint-layout ints, memcpy floats, and scalars in agreement
    // with a full decode — every layout, not just fixed.
    let ints: Vec<i64> = vec![-5, 0, 7, 200, -3000, i64::MAX, i64::MIN];
    let iv = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(ints.clone()))));
    let ib = with_numerics(WireNumerics::Varint, || {
        message_to_wire_with("", &iv, WireCodec::Native, WireIntegrity::Raw).unwrap()
    });
    let view = view_message(&ib).unwrap();
    assert_eq!(view.int_list_len(), Some(ints.len()));
    for (i, &x) in ints.iter().enumerate() {
        assert_eq!(view.int_list_get(i), Some(x), "varint element {i}");
    }
    let flts = vec![1.5f64, -2.25, 3.0e10, f64::MIN, f64::MAX];
    let fv = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(flts.clone()))));
    let fb = message_to_wire_with("", &fv, WireCodec::Native, WireIntegrity::Raw).unwrap();
    let fview = view_message(&fb).unwrap();
    assert_eq!(fview.float_list_len(), Some(flts.len()));
    for (i, &x) in flts.iter().enumerate() {
        assert_eq!(fview.float_list_get(i), Some(x), "float element {i}");
    }
    let sb = message_to_wire_with(
        "",
        &RuntimeValue::Int(-42),
        WireCodec::Native,
        WireIntegrity::Raw,
    )
    .unwrap();
    assert_eq!(view_message(&sb).unwrap().as_int(), Some(-42));
}

#[test]
fn wireview_single_field_read_is_far_cheaper_than_full_decode() {
    // The zero-copy win: reading ONE element of a 1M-element message — even a THOUSAND
    // of them — must be far cheaper than ONE full decode (O(1) access, no bulk copy).
    // A same-run comparison, so it is load-invariant.
    let v: Vec<i64> = (0..1_000_000).map(|i| i as i64).collect();
    let value = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(v))));
    let bytes = with_numerics(WireNumerics::Fixed, || {
        message_to_wire_with("", &value, WireCodec::Native, WireIntegrity::Raw).unwrap()
    });
    let reads = {
        let t = std::time::Instant::now();
        for _ in 0..1000 {
            let view = view_message(&bytes).unwrap();
            std::hint::black_box(view.int_list_get(999_999));
        }
        t.elapsed().as_nanos()
    };
    let full = {
        let t = std::time::Instant::now();
        std::hint::black_box(message_from_wire(&bytes).unwrap());
        t.elapsed().as_nanos()
    };
    assert!(
        reads < full,
        "1000 zero-copy field reads ({reads}ns) must be cheaper than ONE full decode ({full}ns)"
    );
}

#[test]
fn wireview_open_is_o1_even_with_a_checksum() {
    // "Raw for certain": `view_message` never re-hashes the body, so opening a CHECKED
    // message (the default) and reading one element is O(1) — exactly as for a Raw one.
    // (Cap'n Proto / Arrow carry no checksum at all; our zero-copy view matches their
    // cost, while the FULL decode path still validates.) This would FAIL if the view
    // verified the 8 MB body's FNV sum on open.
    let v: Vec<i64> = (0..1_000_000).map(|i| i as i64).collect();
    let value = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(v))));
    let bytes = with_numerics(WireNumerics::Fixed, || message_to_wire("", &value).unwrap());
    assert!(
        bytes[0] & H_CHECKED != 0,
        "the default message carries a checksum"
    );
    let reads = {
        let t = std::time::Instant::now();
        for _ in 0..1000 {
            let view = view_message(&bytes).unwrap();
            std::hint::black_box(view.int_list_get(999_999));
        }
        t.elapsed().as_nanos()
    };
    let full = {
        let t = std::time::Instant::now();
        std::hint::black_box(message_from_wire(&bytes).unwrap());
        t.elapsed().as_nanos()
    };
    assert!(
        reads < full,
        "1000 view reads of a checksummed message ({reads}ns) must beat one full decode ({full}ns)"
    );
}

#[test]
fn wireview_rejects_compressed_and_malformed() {
    // The view is over raw native bytes; a compressed message returns None (it must be
    // inflated first), and a truncated/garbage frame never panics. Use REPETITIVE data
    // so zstd actually shrinks it (the codec keeps compression only when it helps).
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(
        (0..4000).map(|i| (i % 4) as i64 * 1000).collect(),
    ))));
    let compressed = with_compression_codec(WireCompression::Zstd, || {
        message_to_wire_with("", &v, WireCodec::Native, WireIntegrity::Raw).unwrap()
    });
    assert!(
        compressed[0] & 0x02 != 0,
        "the test payload must actually be compressed"
    );
    assert!(
        view_message(&compressed).is_none(),
        "a compressed message has no in-place view"
    );
    assert!(view_message(&[]).is_none(), "empty");
    assert!(
        view_message(&[0xFF, 0x00, 0x01]).is_none(),
        "garbage header"
    );
}

#[test]
fn structure_off_is_the_default_and_leaves_bytes_unchanged() {
    let v: Vec<i64> = (0..50).collect();
    let value = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(v))));
    let default = message_to_wire_with("", &value, WireCodec::Native, WireIntegrity::Raw).unwrap();
    let (off, _) = {
        let b = with_structure(WireStructure::Off, || {
            message_to_wire_with("", &value, WireCodec::Native, WireIntegrity::Raw).unwrap()
        });
        (b, ())
    };
    assert_eq!(
        default, off,
        "Off must equal the default path byte-for-byte (no regression)"
    );
}

/// A BigInt beyond i64 — built straight from a decimal so the test does not depend
/// on the arithmetic layer.
fn big(decimal: &str) -> RuntimeValue {
    RuntimeValue::from_bigint(logicaffeine_base::BigInt::parse_decimal(decimal).unwrap())
}

#[test]
fn bigint_round_trips_through_the_binary_wire_exactly() {
    let v = big("123456789012345678901234567890");
    let bytes = message_to_wire("", &v).unwrap();
    let (_, back) = message_from_wire(&bytes).unwrap();
    assert_eq!(back.to_display_string(), "123456789012345678901234567890");
    assert!(
        matches!(back, RuntimeValue::BigInt(_)),
        "stays a BigInt across the wire"
    );
}

#[test]
fn negative_bigint_round_trips_through_the_wire() {
    let v = big("-99999999999999999999999999999");
    let bytes = message_to_wire("", &v).unwrap();
    let (_, back) = message_from_wire(&bytes).unwrap();
    assert_eq!(back.to_display_string(), "-99999999999999999999999999999");
}

#[test]
fn bigint_round_trips_through_cross_task_materialize_rebuild() {
    let v = big("340282366920938463463374607431768211456"); // 2^128
    let back = rebuild(materialize(&v).unwrap());
    assert_eq!(
        back.to_display_string(),
        "340282366920938463463374607431768211456"
    );
}

/// An exact base-10 fixed-point, built straight from its literal so the test does
/// not depend on the arithmetic layer.
fn dec_lit(s: &str) -> RuntimeValue {
    RuntimeValue::Decimal(Rc::new(logicaffeine_base::Decimal::parse(s).unwrap()))
}

#[test]
fn decimal_round_trips_through_the_binary_wire_preserving_scale() {
    // Money must survive the wire bit-exact, SCALE INCLUDED: `100.00` comes back
    // `100.00`, not `100`, and `0.10` keeps its trailing zero.
    for s in ["19.99", "-0.005", "100.00", "0", "0.10", "123456789.000001"] {
        let bytes = message_to_wire("", &dec_lit(s)).unwrap();
        let (_, back) = message_from_wire(&bytes).unwrap();
        assert!(
            matches!(back, RuntimeValue::Decimal(_)),
            "stays a Decimal across the wire: {s}"
        );
        assert_eq!(
            back.to_display_string(),
            s,
            "scale + value preserved exactly: {s}"
        );
    }
}

#[test]
fn decimal_round_trips_through_cross_task_materialize_rebuild() {
    let v = dec_lit("19.99");
    let back = rebuild(materialize(&v).unwrap());
    assert!(matches!(back, RuntimeValue::Decimal(_)));
    assert_eq!(back.to_display_string(), "19.99");
}

#[test]
fn decimal_money_survives_wire_where_a_json_float_would_drift() {
    // `0.30` crosses the wire bit-exact, unlike the 0.30000000000000004 a JSON/f64
    // consumer would carry — the "JSON numbers ruin lives" footgun, removed for money.
    let bytes = message_to_wire("", &dec_lit("0.30")).unwrap();
    let (_, back) = message_from_wire(&bytes).unwrap();
    assert_eq!(back.to_display_string(), "0.30");
    assert_ne!(0.1_f64 + 0.2, 0.3);
}

/// An exact complex from rational parts (`re_n/re_d + im_n/im_d · i`).
fn cplx(re: (i64, i64), im: (i64, i64)) -> RuntimeValue {
    RuntimeValue::Complex(Rc::new(logicaffeine_base::Complex::new(
        logicaffeine_base::Rational::from_ratio_i64(re.0, re.1).unwrap(),
        logicaffeine_base::Rational::from_ratio_i64(im.0, im.1).unwrap(),
    )))
}

#[test]
fn complex_round_trips_through_the_binary_wire_exactly() {
    // Both rational parts (and `i·i = −1` exactness) survive the wire bit-for-bit.
    for z in [
        cplx((3, 1), (4, 1)),  // 3+4i
        cplx((0, 1), (1, 1)),  // i
        cplx((0, 1), (-1, 1)), // -i
        cplx((-1, 2), (3, 4)), // -1/2 + 3/4 i
        cplx((7, 1), (0, 1)),  // a real (7+0i)
    ] {
        let bytes = message_to_wire("", &z).unwrap();
        let (_, back) = message_from_wire(&bytes).unwrap();
        assert!(
            matches!(back, RuntimeValue::Complex(_)),
            "stays Complex on the wire"
        );
        assert_eq!(back, z, "exact complex survives the wire");
    }
}

#[test]
fn complex_round_trips_through_cross_task_materialize_rebuild() {
    let z = cplx((3, 1), (-4, 1));
    let back = rebuild(materialize(&z).unwrap());
    assert!(matches!(back, RuntimeValue::Complex(_)));
    assert_eq!(back, z);
}

/// A floating-point complex from raw `f64` parts.
fn cplxf(re: f64, im: f64) -> RuntimeValue {
    RuntimeValue::ComplexF(Rc::new(logicaffeine_base::ComplexF::new(re, im)))
}

#[test]
fn complexf_round_trips_through_the_binary_wire_bit_exactly() {
    // Both `f64` parts survive the wire bit-for-bit (16 raw bytes under T_COMPLEXF).
    for z in [
        cplxf(3.0, 4.0),
        cplxf(0.0, 1.0),
        cplxf(-0.5, 0.75),
        cplxf(2.5, 0.0),
        cplxf(1e300, -1e-300),
    ] {
        let bytes = message_to_wire("", &z).unwrap();
        let (_, back) = message_from_wire(&bytes).unwrap();
        assert!(
            matches!(back, RuntimeValue::ComplexF(_)),
            "stays ComplexF on the wire"
        );
        assert_eq!(back, z, "float-complex survives the wire bit-exact");
    }
}

#[test]
fn complexf_round_trips_through_cross_task_materialize_rebuild() {
    let z = cplxf(3.0, -4.0);
    let back = rebuild(materialize(&z).unwrap());
    assert!(matches!(back, RuntimeValue::ComplexF(_)));
    assert_eq!(back, z);
}

fn modu(v: i64, n: i64) -> RuntimeValue {
    RuntimeValue::Modular(Rc::new(logicaffeine_base::Modular::from_i64(v, n).unwrap()))
}

#[test]
fn modular_round_trips_through_the_binary_wire_preserving_the_ring() {
    // Both the residue AND the modulus survive the wire — the ℤ/nℤ ring is preserved.
    for z in [
        modu(3, 7),
        modu(0, 13),
        modu(10, 7),
        modu(123456, 1_000_003),
    ] {
        let bytes = message_to_wire("", &z).unwrap();
        let (_, back) = message_from_wire(&bytes).unwrap();
        assert!(
            matches!(back, RuntimeValue::Modular(_)),
            "stays Modular on the wire"
        );
        assert_eq!(back, z, "exact residue + modulus survive the wire");
    }
}

#[test]
fn modular_round_trips_through_cross_task_materialize_rebuild() {
    let z = modu(42, 97);
    let back = rebuild(materialize(&z).unwrap());
    assert!(matches!(back, RuntimeValue::Modular(_)));
    assert_eq!(back, z);
}

/// A quantity built straight from a magnitude + unit, independent of the arithmetic layer.
fn qty(num: i64, den: i64, unit: &str) -> RuntimeValue {
    let unit = logicaffeine_base::quantity::units::by_name(unit).unwrap();
    let mag = logicaffeine_base::Rational::new(
        logicaffeine_base::BigInt::from_i64(num),
        logicaffeine_base::BigInt::from_i64(den),
    )
    .unwrap();
    RuntimeValue::Quantity(Rc::new(crate::interpreter::QuantityValue {
        q: logicaffeine_base::Quantity::of(mag, &unit),
        unit,
    }))
}

#[test]
fn quantity_round_trips_through_the_binary_wire_exactly() {
    // Value, dimension, AND display unit all survive — including a fractional magnitude
    // (the golden 42/127 ft) and an affine unit (°C, whose offset rides in the rebuilt unit).
    for (q, shown) in [
        (qty(2, 1, "inch"), "2 in"),
        (qty(20, 1, "celsius"), "20 °C"),
        (qty(5, 1, "kilogram"), "5 kg"),
        (qty(42, 127, "foot"), "42/127 ft"),
    ] {
        let bytes = message_to_wire("", &q).unwrap();
        let (_, back) = message_from_wire(&bytes).unwrap();
        assert!(
            matches!(back, RuntimeValue::Quantity(_)),
            "stays a Quantity on the wire: {shown}"
        );
        assert_eq!(
            back.to_display_string(),
            shown,
            "value + unit preserved exactly: {shown}"
        );
        assert_eq!(back, q, "physical equality preserved: {shown}");
    }
}

#[test]
fn quantity_round_trips_through_cross_task_materialize_rebuild() {
    let q = qty(42, 127, "foot");
    let back = rebuild(materialize(&q).unwrap());
    assert!(matches!(back, RuntimeValue::Quantity(_)));
    assert_eq!(back.to_display_string(), "42/127 ft");
    assert_eq!(back, q);
}

// ===================================================================================
// TYPE CONFORMANCE / LOCK-IN HARNESS
//
// The contract every first-class value type must honor, and the lock that makes it
// impossible to add a new `RuntimeValue` variant without deciding its conformance.
// ===================================================================================

/// Conformance class of a runtime value.
#[derive(PartialEq, Eq, Debug)]
enum ConfClass {
    /// A first-class value: must pass FULL conformance — `type_name`, `to_display_string`,
    /// reflexive equality, and lossless wire round-trip (both cross-task and binary).
    Value,
    /// A container of other values (its own round-trip is tested elsewhere; here we only
    /// require name/display/eq so the harness stays representation-agnostic).
    Container,
    /// An opaque handle or reference with no value-semantics wire identity.
    Opaque,
}

/// THE LOCK. This match is exhaustive, so a NEW `RuntimeValue` variant will not compile until it
/// is classified here — forcing a deliberate decision about its conformance. (Completeness
/// Doctrine: no type enters the language without passing through the conformance harness.)
fn conformance_class(v: &RuntimeValue) -> ConfClass {
    match v {
        RuntimeValue::Int(_)
        | RuntimeValue::BigInt(_)
        | RuntimeValue::Rational(_)
        | RuntimeValue::Decimal(_)
        | RuntimeValue::Complex(_)
        | RuntimeValue::ComplexF(_)
        | RuntimeValue::Modular(_)
        | RuntimeValue::Float(_)
        | RuntimeValue::Bool(_)
        | RuntimeValue::Text(_)
        | RuntimeValue::Char(_)
        | RuntimeValue::Nothing
        | RuntimeValue::Duration(_)
        | RuntimeValue::Date(_)
        | RuntimeValue::Moment(_)
        | RuntimeValue::Span { .. }
        | RuntimeValue::Time(_)
        | RuntimeValue::Word(_)
        | RuntimeValue::Quantity(_)
        | RuntimeValue::Money(_)
        | RuntimeValue::Uuid(_)
        | RuntimeValue::Algebraic(_)
        | RuntimeValue::ExactReal(_)
        | RuntimeValue::Hyperreal(_)
        | RuntimeValue::Hypernatural(_)
        | RuntimeValue::Ordinal(_)
        | RuntimeValue::Depends(_)
        | RuntimeValue::Converging(_)
        | RuntimeValue::Peer(_) => ConfClass::Value,
        RuntimeValue::List(_)
        | RuntimeValue::Tuple(_)
        | RuntimeValue::Set(_)
        | RuntimeValue::Map(_)
        | RuntimeValue::Struct(_)
        | RuntimeValue::Inductive(_) => ConfClass::Container,
        RuntimeValue::Function(_)
        | RuntimeValue::Chan(_)
        | RuntimeValue::TaskHandle(_)
        | RuntimeValue::Crdt(_)
        // A SIMD lane vector is a transient compute register: first-class at the AOT tier (the
        // crypto kernels compute over it), but NOT a wire type — you serialize the underlying
        // `Seq`, never the register. So it is not wire-round-tripped (hence Opaque here), yet it
        // IS AOT-wired to a concrete Rust type (declared in `aot_wiring` below).
        | RuntimeValue::Lanes(_)
        // A VIEW is a handle into the page that rendered it. It has no wire identity —
        // sending one would alias another task's DOM — so it is Opaque here, and the LGX
        // tier locks (`view_value_lock`, `layout_lock`) are its conformance harness.
        | RuntimeValue::View(_)
        | RuntimeValue::Action(_) => ConfClass::Opaque,
    }
}

/// One representative of every `ConfClass::Value` variant. If you add a value type, the lock above
/// forces you to classify it; add its representative here so the conformance test exercises it.
fn value_representatives() -> Vec<RuntimeValue> {
    vec![
        RuntimeValue::Int(7),
        // A genuine BigInt (2^64, beyond i64) so it does not narrow back to Int.
        RuntimeValue::BigInt(Rc::new(logicaffeine_base::BigInt::from_le_bytes(
            false,
            &[0, 0, 0, 0, 0, 0, 0, 0, 1],
        ))),
        // A non-integer rational (1/2) so it stays Rational.
        RuntimeValue::Rational(Rc::new(
            logicaffeine_base::Rational::from_ratio_i64(1, 2).unwrap(),
        )),
        dec_lit("19.99"),
        cplx((3, 1), (4, 1)),
        cplxf(3.0, -4.0),
        modu(3, 7),
        RuntimeValue::Float(2.5),
        RuntimeValue::Bool(true),
        RuntimeValue::Text(Rc::new("hi".to_string())),
        RuntimeValue::Char('x'),
        RuntimeValue::Nothing,
        RuntimeValue::Duration(10),
        RuntimeValue::Date(19_000),
        RuntimeValue::Moment(123),
        RuntimeValue::Span {
            months: 3,
            days: 14,
        },
        RuntimeValue::Time(99),
        RuntimeValue::Word(logicaffeine_base::WordVal::from_u64(32, 42).unwrap()),
        qty(42, 127, "foot"),
        RuntimeValue::Money(Rc::new(logicaffeine_base::Money::of(
            logicaffeine_base::Decimal::parse("19.99").unwrap(),
            logicaffeine_base::money::currency::by_code("USD").unwrap(),
        ))),
        RuntimeValue::Uuid(Rc::new(
            logicaffeine_base::Uuid::parse("550e8400-e29b-41d4-a716-446655440000").unwrap(),
        )),
        RuntimeValue::Peer(Rc::new("ws://host:9944".to_string())),
        RuntimeValue::Algebraic(Rc::new(logicaffeine_base::algebraic::Algebraic::from_i64(2).sqrt().unwrap())),
        RuntimeValue::ExactReal(Rc::new(
            logicaffeine_base::exact_real::ExactReal::pi()
                .add(&logicaffeine_base::exact_real::ExactReal::e()),
        )),
        crate::semantics::arith::add(
            crate::semantics::exact::new_infinitesimal("dt", 1, &[0]).unwrap(),
            RuntimeValue::Int(3),
        )
        .unwrap(),
        crate::semantics::exact::generator_of(&crate::semantics::exact::new_infinitesimal("dt", 2, &[0]).unwrap()).unwrap(),
        RuntimeValue::Ordinal(Rc::new(
            logicaffeine_base::ordinal::Ordinal::omega().add(&logicaffeine_base::ordinal::Ordinal::one()),
        )),
        crate::semantics::arith::modulo(
            crate::semantics::exact::generator_of(&crate::semantics::exact::new_infinitesimal("dt", 1, &[0]).unwrap()).unwrap(),
            RuntimeValue::Int(2),
        )
        .unwrap(),
        crate::semantics::exact::provisional(RuntimeValue::Bool(false), 1000, 0),
        crate::semantics::arith::add(
            crate::semantics::exact::new_infinitesimal("dt", 1, &[0]).unwrap(),
            RuntimeValue::ExactReal(Rc::new(logicaffeine_base::exact_real::ExactReal::pi())),
        )
        .unwrap(),
        several_infinitesimals_value(),
        logicaffeine_base::tower::real_fn(logicaffeine_base::tower::RealFn::Sqrt, &several_infinitesimals_value())
            .unwrap(),
        logicaffeine_base::tower::real_fn(logicaffeine_base::tower::RealFn::Exp, &several_infinitesimals_value())
            .unwrap(),
        several_residues_value(),
        integer_of_both_value(),
    ]
}

/// `dt` and `dx` much smaller than it, and the reciprocal of each.
fn two_generators() -> (RuntimeValue, RuntimeValue) {
    use crate::semantics::exact::{generator_of, new_infinitesimal, new_ordered_infinitesimal};
    let dt = new_infinitesimal("dt", 1, &[0]).unwrap();
    let dx = new_ordered_infinitesimal("dx", &dt, &RuntimeValue::Nothing, 1, &[0]).unwrap();
    (generator_of(&dt).unwrap(), generator_of(&dx).unwrap())
}

/// `⌈1/dt⌉ mod 2 + ⌈1/dx⌉ mod 2`: a dependence on the coarser residue whose branches depend on the finer.
fn several_residues_value() -> RuntimeValue {
    use crate::semantics::arith::{add, modulo};
    let (h, hx) = two_generators();
    add(modulo(h, RuntimeValue::Int(2)).unwrap(), modulo(hx, RuntimeValue::Int(2)).unwrap()).unwrap()
}

/// `⌈1/dt⌉·⌈1/dx⌉ + ⌈1/(2·dt)⌉`'s floor: an infinite integer of both, by class of `⌈1/dt⌉`.
fn integer_of_both_value() -> RuntimeValue {
    use crate::semantics::arith::{add, floor_divide, multiply};
    let (h, hx) = two_generators();
    add(multiply(h.clone(), hx).unwrap(), floor_divide(h, RuntimeValue::Int(2)).unwrap()).unwrap()
}

/// `(dt + dy)/(dt - dx)` for `dx` much smaller than `dt` and `dy` between the two: a value of three
/// ordered infinitesimals.
fn several_infinitesimals_value() -> RuntimeValue {
    use crate::semantics::arith::{add, exact_divide, subtract};
    use crate::semantics::exact::{new_infinitesimal, new_ordered_infinitesimal};
    let dt = new_infinitesimal("dt", 1, &[0]).unwrap();
    let dx = new_ordered_infinitesimal("dx", &dt, &RuntimeValue::Nothing, 1, &[0]).unwrap();
    let dy = new_ordered_infinitesimal("dy", &dt, &dx, 2, &[0]).unwrap();
    let top = add(dt.clone(), dy).unwrap();
    let bottom = subtract(dt, dx).unwrap();
    exact_divide(top, bottom).unwrap()
}

/// The compiled-tier (AOT) wiring a runtime value's type MUST have.
#[derive(Debug)]
enum AotWiring {
    /// First-class at the AOT tier: the codegen lowers this type's name to this concrete Rust
    /// type (a primitive like `i64`, or a `Logos*` newtype like `LogosMoney`).
    Rust(&'static str),
    /// Deliberately runtime-only: a handle/reference/container with no single AOT value type
    /// (containers lower generically by element; handles never cross into compiled value code).
    RuntimeOnly,
}

/// THE AOT WIRING LOCK. Exhaustive over `RuntimeValue`, so a NEW variant will not compile until
/// its compiled-tier wiring is declared here — you cannot add a value type and forget to wire it
/// into the AOT codegen. The test below then checks the REAL codegen (`map_type_to_rust`) agrees,
/// so a type wired into the runtime + wire but missing from AOT FAILS. (Completeness Doctrine:
/// all types must be wired through EVERY tier, not just the wire.)
fn aot_wiring(v: &RuntimeValue) -> AotWiring {
    use AotWiring::{RuntimeOnly, Rust};
    match v {
        // A BigInt reports type_name "Int" (same logical type, wider repr) → i64 codegen path.
        RuntimeValue::Int(_) | RuntimeValue::BigInt(_) => Rust("i64"),
        RuntimeValue::Rational(_) => Rust("LogosRational"),
        RuntimeValue::Decimal(_) => Rust("LogosDecimal"),
        RuntimeValue::Complex(_) => Rust("LogosComplex"),
        RuntimeValue::ComplexF(_) => Rust("LogosComplexF"),
        RuntimeValue::Modular(_) => Rust("LogosModular"),
        RuntimeValue::Float(_) => Rust("f64"),
        RuntimeValue::Bool(_) => Rust("bool"),
        RuntimeValue::Text(_) => Rust("String"),
        RuntimeValue::Char(_) => Rust("char"),
        RuntimeValue::Nothing => Rust("()"),
        RuntimeValue::Duration(_) => Rust("std::time::Duration"),
        RuntimeValue::Date(_) => Rust("LogosDate"),
        RuntimeValue::Moment(_) => Rust("LogosMoment"),
        RuntimeValue::Span { .. } => Rust("LogosSpan"),
        RuntimeValue::Time(_) => Rust("LogosTime"),
        // The representative is a 32-bit word; its type_name "Word32" is in the codegen map.
        RuntimeValue::Word(_) => Rust("Word32"),
        // A lane vector is AOT-first-class (the crypto kernels' working registers) even though it
        // is not wire-round-tripped — `map_type_to_rust("Lanes8Word32")` lowers it.
        RuntimeValue::Lanes(_) => Rust("Lanes8Word32"),
        RuntimeValue::Quantity(_) => Rust("LogosQuantity"),
        RuntimeValue::Money(_) => Rust("LogosMoney"),
        RuntimeValue::Uuid(_) => Rust("LogosUuid"),
        // The exact types are values of one compiled model, which keeps each in its canonical form.
        RuntimeValue::Algebraic(_)
        | RuntimeValue::ExactReal(_)
        | RuntimeValue::Hyperreal(_)
        | RuntimeValue::Hypernatural(_)
        | RuntimeValue::Ordinal(_) => Rust("LogosValue"),
        // A residue-dependent value and a provisional answer wrap a value of another type; like a
        // container, each lowers generically by what it holds rather than by a bare name.
        RuntimeValue::Depends(_) | RuntimeValue::Converging(_) => RuntimeOnly,
        // A peer is a runtime networking handle — it round-trips on the wire but is not a
        // compiled-tier value type.
        RuntimeValue::Peer(_) => RuntimeOnly,
        // Containers lower generically (LogosSeq<T>/LogosMap<K,V>/…) via codegen_type_expr, not by
        // a bare name; their own AOT coverage is the collection codegen tests.
        RuntimeValue::List(_)
        | RuntimeValue::Tuple(_)
        | RuntimeValue::Set(_)
        | RuntimeValue::Map(_)
        | RuntimeValue::Struct(_)
        | RuntimeValue::Inductive(_) => RuntimeOnly,
        // Opaque handles never cross into compiled value code. A view is the LGX web
        // surface: the tree-walker renders it, and the AOT tiers refuse it loudly
        // (`refuses_the_unlowered_lgx_constructs_loudly`) rather than lowering a DOM handle.
        RuntimeValue::View(_)
        | RuntimeValue::Action(_)
        | RuntimeValue::Function(_)
        | RuntimeValue::Chan(_)
        | RuntimeValue::TaskHandle(_)
        | RuntimeValue::Crdt(_) => RuntimeOnly,
    }
}

/// Full conformance for one value: name, display, reflexive equality, and BOTH wire round-trips.
fn assert_value_conformance(v: &RuntimeValue) {
    let name = v.type_name();
    assert!(!name.is_empty(), "type_name must be non-empty");
    // Display must not panic.
    let _ = v.to_display_string();
    // Equality is reflexive (representatives are finite — no NaN).
    assert!(v == v, "{name} value is not equal to itself");
    // Cross-task marshalling (materialize → rebuild) is the identity.
    let back = rebuild(materialize(v).unwrap_or_else(|_| panic!("{name} did not materialize")));
    assert_eq!(&back, v, "{name} changed across cross-task marshalling");
    assert_eq!(
        conformance_class(&back),
        ConfClass::Value,
        "{name} changed variant across marshalling"
    );
    // Binary wire (encode → decode) is the identity.
    let bytes = message_to_wire("", v).unwrap_or_else(|_| panic!("{name} did not encode to wire"));
    let (_, back2) = message_from_wire(&bytes).unwrap_or_else(|| panic!("{name} did not decode"));
    assert_eq!(&back2, v, "{name} changed across the binary wire");
}

#[test]
fn every_value_type_conforms_name_display_eq_and_both_wire_round_trips() {
    let reps = value_representatives();
    for v in &reps {
        assert_eq!(
            conformance_class(v),
            ConfClass::Value,
            "representative for {} must be a Value",
            v.type_name()
        );
        assert_value_conformance(v);
    }
    // Sanity: the representative set is non-trivial and covers the value types we have shipped.
    assert!(
        reps.len() >= 20,
        "expected a representative for every value type"
    );
}

#[test]
fn every_value_type_is_wired_into_the_aot_codegen_tier() {
    // A first-class value must compile to Rust, not just travel the wire. For every Value
    // representative whose AOT wiring spec names a concrete Rust type, the real codegen
    // (`map_type_to_rust`, the single source of truth for type-name → Rust) MUST lower the
    // value's `type_name()` to exactly that. Adding a value type wired into the runtime + wire
    // but missing from the AOT codegen fails here — the gap the wire conformance test alone
    // could not catch. `RuntimeOnly` is the conscious, exhaustive-match-forced exemption (e.g. a
    // networking handle); the exhaustiveness of `aot_wiring` is what makes the decision
    // unavoidable for every new type.
    let mut checked = 0usize;
    for v in &value_representatives() {
        let name = v.type_name();
        if let AotWiring::Rust(expected) = aot_wiring(v) {
            let mapped = crate::codegen::types::map_type_to_rust(name);
            assert_eq!(
                mapped, expected,
                "{name}: AOT codegen lowers it to `{mapped}`, but the wiring lock expects `{expected}` \
                     — wire the type into codegen/types.rs::map_type_to_rust"
            );
            // ANALYSIS tier: where the type checker knows this type name, its lowering MUST agree
            // with the codegen — the two type-lowering paths cannot drift. (A type the checker does
            // not yet model by name, e.g. the width-indexed `Word`, returns `Unknown` and is left to
            // the codegen lock above; that exemption is documented, not silent.)
            let lt = crate::analysis::types::LogosType::from_type_name(name);
            if lt != crate::analysis::types::LogosType::Unknown {
                assert_eq!(
                    lt.to_rust_type(), expected,
                    "{name}: analysis LogosType lowers to `{}`, codegen to `{expected}` — the type checker \
                         and codegen disagree; wire them consistently",
                    lt.to_rust_type()
                );
            }
            checked += 1;
        }
    }
    // The lock must actually be exercising the AOT tier, not silently skipping everything.
    assert!(
        checked >= 18,
        "AOT wiring lock checked only {checked} value types"
    );
}

#[test]
fn compound_dimension_quantity_survives_the_wire_as_its_signature() {
    // A dimension-combining product (Area, empty display symbol) preserves its dimension and
    // renders the signature on the far side.
    let m = logicaffeine_base::quantity::units::by_name("meter").unwrap();
    let area = logicaffeine_base::Quantity::of(logicaffeine_base::Rational::from_i64(3), &m).mul(
        &logicaffeine_base::Quantity::of(logicaffeine_base::Rational::from_i64(4), &m),
    );
    let area_unit =
        logicaffeine_base::Unit::linear("", area.dimension(), logicaffeine_base::Rational::one());
    let v = RuntimeValue::Quantity(Rc::new(crate::interpreter::QuantityValue {
        q: area,
        unit: area_unit,
    }));
    let (_, back) = message_from_wire(&message_to_wire("", &v).unwrap()).unwrap();
    assert_eq!(back.to_display_string(), "12 L^2");
    assert_eq!(back, v);
}

#[test]
fn our_wire_preserves_an_integer_that_json_would_corrupt() {
    // 2^53 + 1 is the smallest integer a conforming JSON (f64) consumer rounds
    // away — the canonical "JSON numbers ruin lives" value. Our typed wire keeps
    // the i64 EXACT, with no 2^53 cliff.
    let n = 9_007_199_254_740_993i64;
    let bytes = message_to_wire("", &RuntimeValue::Int(n)).unwrap();
    let (_, back) = message_from_wire(&bytes).unwrap();
    assert_eq!(back, RuntimeValue::Int(n), "our wire keeps it exact");
    // The JSON number model (IEEE-754 double) loses it.
    assert_ne!(
        n as f64 as i64, n,
        "f64 round-trip corrupts 2^53+1 — the JSON footgun"
    );
}

fn rat(n: i64, d: i64) -> RuntimeValue {
    RuntimeValue::from_rational(logicaffeine_base::Rational::from_ratio_i64(n, d).unwrap())
}

#[test]
fn our_wire_preserves_a_fraction_that_json_would_round() {
    // 1/3 has no exact f64 / JSON-number representation — a JSON consumer stores
    // 0.3333… and can never recover 1/3. Our typed wire keeps the fraction EXACT —
    // the other half of "JSON numbers ruin lives", alongside the 2^53 cliff above.
    let bytes = message_to_wire("", &rat(1, 3)).unwrap();
    let (_, back) = message_from_wire(&bytes).unwrap();
    assert_eq!(
        back.to_display_string(),
        "1/3",
        "the fraction survives the wire exactly"
    );
    assert!(
        matches!(back, RuntimeValue::Rational(_)),
        "stays a Rational across the wire"
    );
    // The JSON/IEEE-754 model can't even add tenths exactly — the footgun this removes.
    assert_ne!(0.1_f64 + 0.2, 0.3);
}

#[test]
fn rational_round_trips_through_the_wire_and_cross_task() {
    // Includes a fraction that REDUCES to a whole number (6/2 → 3): it downsizes to
    // an Int and rides the wire as one, proving the canonical-representation invariant.
    for (n, d, shown) in [
        (7i64, 2i64, "7/2"),
        (-3, 4, "-3/4"),
        (6, 2, "3"),
        (22, 7, "22/7"),
    ] {
        let v = rat(n, d);
        let bytes = message_to_wire("", &v).unwrap();
        let (_, back) = message_from_wire(&bytes).unwrap();
        assert_eq!(
            back.to_display_string(),
            shown,
            "{n}/{d} via the binary wire"
        );
        let back2 = rebuild(materialize(&v).unwrap());
        assert_eq!(
            back2.to_display_string(),
            shown,
            "{n}/{d} via materialize/rebuild"
        );
    }
}

#[test]
fn scalars_and_temporals_roundtrip() {
    for v in [
        RuntimeValue::Int(5),
        RuntimeValue::Float(2.5),
        RuntimeValue::Bool(true),
        RuntimeValue::Char('z'),
        RuntimeValue::Nothing,
        RuntimeValue::Duration(10),
        RuntimeValue::Date(19_000),
        RuntimeValue::Moment(123),
        RuntimeValue::Span {
            months: 3,
            days: 14,
        },
        RuntimeValue::Time(99),
    ] {
        assert_roundtrips(&v);
    }
}

#[test]
fn text_roundtrips() {
    let v = RuntimeValue::Text(Rc::new("hello".to_string()));
    let p = assert_roundtrips(&v);
    assert_eq!(p, RtPayload::Text("hello".to_string()));
}

#[test]
fn peer_handle_roundtrips_exactly() {
    let v = RuntimeValue::Peer(Rc::new("ws://127.0.0.1:9944".to_string()));
    let p = assert_roundtrips(&v);
    assert_eq!(p, RtPayload::Peer("ws://127.0.0.1:9944".to_string()));
    // …and rebuilds back to a Peer (not a bare Text).
    assert!(matches!(rebuild(p), RuntimeValue::Peer(t) if t.as_str() == "ws://127.0.0.1:9944"));
}

#[test]
fn int_list_roundtrips() {
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(vec![
        RuntimeValue::Int(1),
        RuntimeValue::Int(2),
        RuntimeValue::Int(3),
    ]))));
    let p = assert_roundtrips(&v);
    assert_eq!(
        p,
        RtPayload::List(vec![
            RtPayload::Int(1),
            RtPayload::Int(2),
            RtPayload::Int(3)
        ])
    );
}

#[test]
fn set_and_tuple_roundtrip() {
    let set = RuntimeValue::Set(Rc::new(RefCell::new(vec![
        RuntimeValue::Int(7),
        RuntimeValue::Int(8),
    ])));
    let p = assert_roundtrips(&set);
    assert_eq!(
        p,
        RtPayload::Set(vec![RtPayload::Int(7), RtPayload::Int(8)])
    );

    let tup = RuntimeValue::Tuple(Rc::new(vec![
        RuntimeValue::Int(1),
        RuntimeValue::Bool(false),
        RuntimeValue::Char('x'),
    ]));
    let p = assert_roundtrips(&tup);
    assert_eq!(
        p,
        RtPayload::Tuple(vec![
            RtPayload::Int(1),
            RtPayload::Bool(false),
            RtPayload::Char('x')
        ])
    );
}

#[test]
fn sets_are_canonical_regardless_of_insertion_order() {
    // SYMMETRY BREAKING ON THE WIRE: a set is order-invariant, so the SAME members in ANY
    // insertion order MUST serialize to byte-identical wire — so content-addressing, dedup,
    // `Send cached` keying, and FEC all hold for sets. Mirrors canonical T_MAP / T_STRUCT.
    let mk = |order: &[i64]| {
        RuntimeValue::Set(Rc::new(RefCell::new(
            order
                .iter()
                .map(|&n| RuntimeValue::Int(n))
                .collect::<Vec<_>>(),
        )))
    };
    let mut e1 = Vec::new();
    native_encode(&mk(&[5, 1, 3, 2, 4]), &mut e1).unwrap();
    let mut e2 = Vec::new();
    native_encode(&mk(&[4, 2, 3, 1, 5]), &mut e2).unwrap();
    let mut e3 = Vec::new();
    native_encode(&mk(&[1, 2, 3, 4, 5]), &mut e3).unwrap();
    assert_eq!(
        e1, e2,
        "same set, different insertion order → byte-identical wire"
    );
    assert_eq!(
        e1, e3,
        "...identical to the already-sorted insertion order too"
    );

    let mut pos = 0;
    match native_decode(&e1, &mut pos).expect("decode") {
        RuntimeValue::Set(s) => {
            let mut got: Vec<i64> = s
                .borrow()
                .iter()
                .map(|v| match v {
                    RuntimeValue::Int(n) => *n,
                    other => panic!("expected Int in set, got {other:?}"),
                })
                .collect();
            got.sort_unstable();
            assert_eq!(
                got,
                vec![1, 2, 3, 4, 5],
                "canonical bytes round-trip to the same members"
            );
        }
        other => panic!("expected a Set, got {other:?}"),
    }
}

#[test]
fn int_set_crushes_to_a_compressed_column() {
    // The extreme symmetry-break case: a CONSECUTIVE int set {1..1000} (built in reverse order).
    // Sorted, it's a perfect affine column → base+stride+count, ZERO per-element data. It must be
    // a tiny fraction of the ~1000+ bytes the naive tagged-element encoding would write, and round
    // -trip exactly. (A clustered/sparse set instead gets delta/RLE — never worse than the naive.)
    let n = 1000i64;
    let order: Vec<RuntimeValue> = (1..=n).rev().map(RuntimeValue::Int).collect();
    let set = RuntimeValue::Set(Rc::new(RefCell::new(order)));
    let mut enc = Vec::new();
    native_encode(&set, &mut enc).unwrap();
    assert!(
        enc.len() < 64,
        "a consecutive int set of {n} must collapse to a closed-form column, got {} bytes",
        enc.len()
    );
    let mut pos = 0;
    match native_decode(&enc, &mut pos).unwrap() {
        RuntimeValue::Set(s) => {
            let mut got: Vec<i64> = s
                .borrow()
                .iter()
                .map(|v| match v {
                    RuntimeValue::Int(k) => *k,
                    other => panic!("expected Int, got {other:?}"),
                })
                .collect();
            got.sort_unstable();
            assert_eq!(
                got,
                (1..=n).collect::<Vec<_>>(),
                "round-trips to the full set"
            );
        }
        other => panic!("expected a Set, got {other:?}"),
    }
}

#[test]
fn int_keyed_map_columnarizes_keys_and_values() {
    // The map analog of the int-set crush. A map with CONSECUTIVE integer keys {0..1000} mapping
    // to an affine value {i ↦ 2i} (inserted in REVERSE) ships as TWO columns: the keys collapse to
    // an affine closed form (base+stride+count, no per-key data) AND the values collapse the same
    // way (they reuse the full best-list column menu). So the whole int→int map — 1000 entries —
    // becomes a handful of bytes, vs the ~5–6 KB the per-entry `T_MAP` would write (each key AND
    // value a tagged inline varint). Must round-trip exactly and be insertion-order-invariant.
    let n = 1000i64;
    let build = |rev: bool| {
        let mut keys: Vec<i64> = (0..n).collect();
        if rev {
            keys.reverse();
        }
        let mut m = MapStorage::default();
        for k in keys {
            m.insert(RuntimeValue::Int(k), RuntimeValue::Int(k * 2));
        }
        RuntimeValue::Map(Rc::new(RefCell::new(m)))
    };

    let mut enc = Vec::new();
    native_encode(&build(false), &mut enc).unwrap();
    assert!(
        enc.len() < 64,
        "an affine int→int map of {n} entries must collapse BOTH columns to closed forms, got {} bytes",
        enc.len()
    );

    // Canonical: the SAME map built in the opposite insertion order is BYTE-IDENTICAL.
    let mut enc_rev = Vec::new();
    native_encode(&build(true), &mut enc_rev).unwrap();
    assert_eq!(
        enc, enc_rev,
        "int-keyed map encoding must be insertion-order-invariant"
    );

    // Round-trips exactly.
    let mut pos = 0;
    match native_decode(&enc, &mut pos).unwrap() {
        RuntimeValue::Map(m) => {
            let b = m.borrow();
            assert_eq!(b.len(), n as usize, "all entries recovered");
            for k in 0..n {
                let got = b.get(&RuntimeValue::Int(k)).expect("key present");
                assert_eq!(*got, RuntimeValue::Int(k * 2), "value for key {k} survived");
            }
        }
        other => panic!("expected a Map, got {other:?}"),
    }
}

#[test]
fn int_keyed_map_with_non_int_values_still_columnarizes_keys() {
    // Keys all Int but values heterogeneous-ish (Text) — the KEY column still collapses; the value
    // list takes its own best encoding (here a string column). Must round-trip + stay canonical.
    let build = |rev: bool| {
        let mut ks: Vec<i64> = vec![10, 20, 30, 40];
        if rev {
            ks.reverse();
        }
        let mut m = MapStorage::default();
        for k in ks {
            m.insert(
                RuntimeValue::Int(k),
                RuntimeValue::Text(Rc::new(format!("item_{k}"))),
            );
        }
        RuntimeValue::Map(Rc::new(RefCell::new(m)))
    };
    let mut a = Vec::new();
    native_encode(&build(false), &mut a).unwrap();
    let mut b = Vec::new();
    native_encode(&build(true), &mut b).unwrap();
    assert_eq!(
        a, b,
        "int-keyed map with text values must be insertion-order-invariant"
    );
    let mut pos = 0;
    match native_decode(&a, &mut pos).unwrap() {
        RuntimeValue::Map(m) => {
            let mb = m.borrow();
            assert_eq!(mb.len(), 4);
            for k in [10i64, 20, 30, 40] {
                let got = mb.get(&RuntimeValue::Int(k)).expect("key present");
                assert_eq!(*got, RuntimeValue::Text(Rc::new(format!("item_{k}"))));
            }
        }
        other => panic!("expected a Map, got {other:?}"),
    }
}

#[test]
fn int_keyed_map_front_codes_string_value_column() {
    // The database-table case: int primary key → string column with shared prefixes (URLs/paths/
    // ids). Keys collapse to an affine column; the VALUES front-code in key order (each shipped as
    // shared-prefix-len-with-previous + suffix), so a column of `https://…/items/<i>` ships only
    // the changing suffix. Must be far smaller than the per-value fallback, round-trip, be canonical.
    let n = 100i64;
    let build = |rev: bool| {
        let mut ks: Vec<i64> = (0..n).collect();
        if rev {
            ks.reverse();
        }
        let mut m = MapStorage::default();
        for k in ks {
            m.insert(
                RuntimeValue::Int(k),
                RuntimeValue::Text(Rc::new(format!("https://example.com/items/{k}"))),
            );
        }
        RuntimeValue::Map(Rc::new(RefCell::new(m)))
    };

    let mut enc = Vec::new();
    native_encode(&build(false), &mut enc).unwrap();
    // Per-value (kind 0) would ship ~30 bytes × 100 ≈ 3 KB; front-coding the ~26-char shared prefix
    // leaves only the suffix deltas.
    assert!(
        enc.len() < 1200,
        "an int→string map with a shared value prefix must front-code its value column, got {} bytes",
        enc.len()
    );

    let mut enc_rev = Vec::new();
    native_encode(&build(true), &mut enc_rev).unwrap();
    assert_eq!(
        enc, enc_rev,
        "int→string map must be insertion-order-invariant"
    );

    let mut pos = 0;
    match native_decode(&enc, &mut pos).unwrap() {
        RuntimeValue::Map(m) => {
            let b = m.borrow();
            assert_eq!(b.len(), n as usize);
            for k in 0..n {
                let got = b.get(&RuntimeValue::Int(k)).expect("key present");
                assert_eq!(
                    *got,
                    RuntimeValue::Text(Rc::new(format!("https://example.com/items/{k}"))),
                    "value for key {k} survived"
                );
            }
        }
        other => panic!("expected a Map, got {other:?}"),
    }
}

#[test]
fn int_keyed_map_columnarizes_struct_value_column() {
    // The full database-table crush: int primary key → homogeneous struct record (id/name/active).
    // The values pack COLUMNAR like a struct list — the field NAMES ship ONCE, then one column per
    // field (the id column compresses, the active column bit-packs) — instead of re-shipping the
    // whole self-describing struct per row (the per-value fallback). Must round-trip + be canonical.
    let n = 100i64;
    let rec = |id: i64| {
        let mut f = HashMap::new();
        f.insert("id".to_string(), RuntimeValue::Int(id));
        f.insert(
            "name".to_string(),
            RuntimeValue::Text(Rc::new(format!("user_{id}"))),
        );
        f.insert("active".to_string(), RuntimeValue::Bool(id % 2 == 0));
        RuntimeValue::Struct(Box::new(StructValue {
            type_name: "Rec".to_string(),
            fields: f,
        }))
    };
    let build = |rev: bool| {
        let mut ks: Vec<i64> = (0..n).collect();
        if rev {
            ks.reverse();
        }
        let mut m = MapStorage::default();
        for k in ks {
            m.insert(RuntimeValue::Int(k), rec(k));
        }
        RuntimeValue::Map(Rc::new(RefCell::new(m)))
    };

    let mut enc = Vec::new();
    native_encode(&build(false), &mut enc).unwrap();
    // Per-value (kind 0) re-ships "Rec"+"id"+"name"+"active" per row ≈ 27 B × 100 ≈ 2.7 KB; the
    // columnar form ships the schema once.
    assert!(
        enc.len() < 1500,
        "an int→struct map must pack its value column columnarly (schema once), got {} bytes",
        enc.len()
    );

    let mut enc_rev = Vec::new();
    native_encode(&build(true), &mut enc_rev).unwrap();
    assert_eq!(
        enc, enc_rev,
        "int→struct map must be insertion-order-invariant"
    );

    // Round-trip proof by BYTE-STABILITY (re-encode the decoded value and compare bytes), the
    // canonical method in this codec — `RuntimeValue`'s `PartialEq` does not deep-compare struct
    // values, so direct `assert_eq!` on a decoded struct is unavailable; byte-identity is strictly
    // stronger (any dropped/corrupted field would perturb the re-encoding).
    let mut pos = 0;
    let decoded = native_decode(&enc, &mut pos).unwrap();
    match &decoded {
        RuntimeValue::Map(m) => assert_eq!(m.borrow().len(), n as usize, "all rows recovered"),
        other => panic!("expected a Map, got {other:?}"),
    }
    let mut reenc = Vec::new();
    native_encode(&decoded, &mut reenc).unwrap();
    assert_eq!(
        reenc, enc,
        "decoded int→struct map must re-encode byte-identically"
    );
}

#[test]
fn string_set_front_codes_shared_prefixes() {
    // Sorted strings share prefixes → front-coding ships only the suffix deltas. A set of strings
    // with long common prefixes must be smaller than the naive concat, round-trip exactly, and be
    // canonical (insertion-order-invariant).
    let words = [
        "user_1000",
        "user_1001",
        "user_1002",
        "user_1003",
        "user_2000",
    ];
    let mk = |order: &[usize]| {
        RuntimeValue::Set(Rc::new(RefCell::new(
            order
                .iter()
                .map(|&i| RuntimeValue::Text(Rc::new(words[i].to_string())))
                .collect::<Vec<_>>(),
        )))
    };
    let mut e1 = Vec::new();
    native_encode(&mk(&[0, 1, 2, 3, 4]), &mut e1).unwrap();
    let mut e2 = Vec::new();
    native_encode(&mk(&[4, 2, 0, 3, 1]), &mut e2).unwrap();
    assert_eq!(
        e1, e2,
        "same string set, different insertion order → identical wire"
    );

    let naive: usize = words.iter().map(|w| w.len()).sum();
    assert!(
        e1.len() < naive,
        "front-coding must beat the {naive}-byte naive concat (shared prefixes elided), got {}",
        e1.len()
    );

    let mut pos = 0;
    match native_decode(&e1, &mut pos).unwrap() {
        RuntimeValue::Set(s) => {
            let mut got: Vec<String> = s
                .borrow()
                .iter()
                .map(|v| match v {
                    RuntimeValue::Text(t) => (**t).clone(),
                    other => panic!("expected Text, got {other:?}"),
                })
                .collect();
            got.sort();
            let mut want: Vec<String> = words.iter().map(|w| w.to_string()).collect();
            want.sort();
            assert_eq!(got, want, "round-trips to the same members");
        }
        other => panic!("expected a Set, got {other:?}"),
    }
}

#[test]
fn single_entry_map_roundtrips_exactly() {
    let mut m: MapStorage = MapStorage::default();
    m.insert(
        RuntimeValue::Int(1),
        RuntimeValue::Text(Rc::new("a".to_string())),
    );
    let v = RuntimeValue::Map(Rc::new(RefCell::new(m)));
    let p = assert_roundtrips(&v);
    assert_eq!(
        p,
        RtPayload::Map(vec![(RtPayload::Int(1), RtPayload::Text("a".to_string()))])
    );
}

#[test]
fn multi_entry_map_preserves_entries() {
    let mut m: MapStorage = MapStorage::default();
    m.insert(RuntimeValue::Int(1), RuntimeValue::Int(10));
    m.insert(RuntimeValue::Int(2), RuntimeValue::Int(20));
    let v = RuntimeValue::Map(Rc::new(RefCell::new(m)));
    let p = materialize(&v).unwrap();
    let entries = match &p {
        RtPayload::Map(e) => e,
        _ => panic!("expected map"),
    };
    assert_eq!(entries.len(), 2);
    assert!(entries.contains(&(RtPayload::Int(1), RtPayload::Int(10))));
    assert!(entries.contains(&(RtPayload::Int(2), RtPayload::Int(20))));

    // Round-trip preserves the entry set (order may differ — hashmap).
    let p2 = materialize(&rebuild(p.clone())).unwrap();
    let e2 = match &p2 {
        RtPayload::Map(e) => e,
        _ => panic!("expected map"),
    };
    assert_eq!(entries.len(), e2.len());
    for e in entries {
        assert!(e2.contains(e), "entry {e:?} lost in round-trip");
    }
}

#[test]
fn single_field_struct_roundtrips_exactly() {
    let mut fields = HashMap::new();
    fields.insert("x".to_string(), RuntimeValue::Int(7));
    let v = RuntimeValue::Struct(Box::new(StructValue {
        type_name: "Point".to_string(),
        fields,
    }));
    let p = assert_roundtrips(&v);
    assert_eq!(
        p,
        RtPayload::Struct {
            type_name: "Point".to_string(),
            fields: vec![("x".to_string(), RtPayload::Int(7))],
        }
    );
}

#[test]
fn inductive_roundtrips() {
    let v = RuntimeValue::Inductive(std::rc::Rc::new(InductiveValue {
        inductive_type: "Option".to_string(),
        constructor: "Some".to_string(),
        args: vec![RuntimeValue::Int(42)],
    }));
    let p = assert_roundtrips(&v);
    assert_eq!(
        p,
        RtPayload::Inductive {
            type_name: "Option".to_string(),
            constructor: "Some".to_string(),
            args: vec![RtPayload::Int(42)],
        }
    );
}

#[test]
fn nested_collections_roundtrip() {
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(vec![
        RuntimeValue::Tuple(Rc::new(vec![
            RuntimeValue::Int(1),
            RuntimeValue::Bool(true),
        ])),
        RuntimeValue::Text(Rc::new("nested".to_string())),
    ]))));
    assert_roundtrips(&v);
}

// ── Best-of-both-worlds interop: the peer codec (RuntimeValue) and the shared wire core
//    (an AOT-generated-style type) must produce and read the SAME bytes. This is the lock
//    that lets a compile-once native PE receive a program over the real fast codec. ──────
use logicaffeine_data::wire::{self, WireDecode, WireEncode};

// A hand-written mirror of exactly what codegen emits for a `## A CE is one of:` enum.
#[derive(Debug, Clone, PartialEq)]
enum CE {
    CInt(i64),
    CText(String),
    CBool(bool),
    CBinOp {
        op: String,
        left: Box<CE>,
        right: Box<CE>,
    },
    CList(Vec<CE>),
}
impl WireEncode for CE {
    fn wire_encode(&self, out: &mut Vec<u8>) {
        match self {
            CE::CInt(v) => {
                wire::write_inductive_header(out, "CE", "CInt", 1);
                v.wire_encode(out);
            }
            CE::CText(s) => {
                wire::write_inductive_header(out, "CE", "CText", 1);
                s.wire_encode(out);
            }
            CE::CBool(b) => {
                wire::write_inductive_header(out, "CE", "CBool", 1);
                b.wire_encode(out);
            }
            CE::CBinOp { op, left, right } => {
                wire::write_inductive_header(out, "CE", "CBinOp", 3);
                op.wire_encode(out);
                left.wire_encode(out);
                right.wire_encode(out);
            }
            CE::CList(xs) => {
                wire::write_inductive_header(out, "CE", "CList", 1);
                xs.wire_encode(out);
            }
        }
    }
}
impl WireDecode for CE {
    fn wire_decode(buf: &[u8], pos: &mut usize) -> Option<Self> {
        let (ty, ctor, _n) = wire::read_inductive_header(buf, pos)?;
        debug_assert_eq!(ty, "CE");
        Some(match ctor.as_str() {
            "CInt" => CE::CInt(i64::wire_decode(buf, pos)?),
            "CText" => CE::CText(String::wire_decode(buf, pos)?),
            "CBool" => CE::CBool(bool::wire_decode(buf, pos)?),
            "CBinOp" => CE::CBinOp {
                op: String::wire_decode(buf, pos)?,
                left: Box::<CE>::wire_decode(buf, pos)?,
                right: Box::<CE>::wire_decode(buf, pos)?,
            },
            "CList" => CE::CList(Vec::<CE>::wire_decode(buf, pos)?),
            _ => return None,
        })
    }
}

// The same logical program in both value models.
fn ce_tree() -> CE {
    CE::CBinOp {
        op: "+".to_string(),
        left: Box::new(CE::CInt(2)),
        right: Box::new(CE::CList(vec![
            CE::CInt(3),
            CE::CBool(true),
            CE::CText("hi".to_string()),
        ])),
    }
}
fn rt_ind(ty: &str, ctor: &str, args: Vec<RuntimeValue>) -> RuntimeValue {
    RuntimeValue::Inductive(std::rc::Rc::new(InductiveValue {
        inductive_type: ty.to_string(),
        constructor: ctor.to_string(),
        args,
    }))
}
fn rt_tree() -> RuntimeValue {
    rt_ind(
        "CE",
        "CBinOp",
        vec![
            RuntimeValue::Text(Rc::new("+".to_string())),
            rt_ind("CE", "CInt", vec![RuntimeValue::Int(2)]),
            rt_ind(
                "CE",
                "CList",
                vec![RuntimeValue::List(Rc::new(RefCell::new(
                    ListRepr::from_values(vec![
                        rt_ind("CE", "CInt", vec![RuntimeValue::Int(3)]),
                        rt_ind("CE", "CBool", vec![RuntimeValue::Bool(true)]),
                        rt_ind(
                            "CE",
                            "CText",
                            vec![RuntimeValue::Text(Rc::new("hi".to_string()))],
                        ),
                    ]),
                )))],
            ),
        ],
    )
}

#[test]
fn peer_and_wire_core_produce_identical_bytes() {
    let peer_bytes = encode_value_raw(&rt_tree()).expect("peer encode");
    let mut wire_bytes = Vec::new();
    ce_tree().wire_encode(&mut wire_bytes);
    assert_eq!(
        peer_bytes, wire_bytes,
        "the shared wire core must be byte-identical to the peer codec"
    );
}

#[test]
fn peer_encode_then_wire_decode_reconstructs_the_generated_type() {
    // The exact path a compile-once native PE uses: host peer-encodes, native wire-decodes.
    let bytes = encode_value_raw(&rt_tree()).expect("peer encode");
    let mut pos = 0usize;
    let decoded = CE::wire_decode(&bytes, &mut pos).expect("wire decode");
    assert_eq!(pos, bytes.len(), "must consume every byte");
    assert_eq!(decoded, ce_tree());
}

#[test]
fn wire_encode_then_peer_decode_reconstructs_the_runtime_value() {
    // The reverse direction: native wire-encodes, host peer-decodes.
    let mut bytes = Vec::new();
    ce_tree().wire_encode(&mut bytes);
    let rv = decode_value_raw(&bytes).expect("peer decode");
    assert_eq!(materialize(&rv).unwrap(), materialize(&rt_tree()).unwrap());
}

#[test]
fn function_is_not_sendable() {
    let v = RuntimeValue::Function(Box::new(ClosureValue {
        body_index: 0,
        captured_env: HashMap::default(),
        param_names: vec![],
        generated: None,
    }));
    assert_eq!(materialize(&v), Err(MarshalError::NotSendable("Function")));
}

// -------------------------------------------------------------------------
// Wire codec — a message is any language value
// -------------------------------------------------------------------------

/// A value survives the wire iff its `RtPayload` is unchanged across
/// encode→decode (we compare through `RtPayload`, which has structural eq).
fn assert_wire_roundtrips(v: &RuntimeValue, from: &str) {
    let bytes = message_to_wire(from, v).expect("message_to_wire");
    let (got_from, back) = message_from_wire(&bytes).expect("message_from_wire");
    assert_eq!(got_from, from, "sender lost on the wire");
    assert_eq!(
        materialize(v).expect("before"),
        materialize(&back).expect("after"),
        "wire round-trip changed the value"
    );
}

#[test]
fn message_wire_scalars_roundtrip() {
    for v in [
        RuntimeValue::Int(42),
        RuntimeValue::Float(2.5),
        RuntimeValue::Bool(true),
        RuntimeValue::Char('z'),
        RuntimeValue::Text(Rc::new("ping".to_string())),
        RuntimeValue::Nothing,
        RuntimeValue::Duration(1000),
    ] {
        assert_wire_roundtrips(&v, "alice");
    }
}

#[test]
fn message_wire_is_compact_binary() {
    // A 100-element int list encodes to compact binary (a few bytes/element),
    // far tighter than a text encoding, and round-trips exactly.
    let items: Vec<RuntimeValue> = (0..100).map(RuntimeValue::Int).collect();
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(items))));
    let bytes = message_to_wire("", &v).unwrap();
    assert!(
        bytes.len() < 100 * 12,
        "wire should be compact, was {} bytes",
        bytes.len()
    );
    assert_wire_roundtrips(&v, "");
}

#[test]
fn message_wire_anonymous_sender_is_empty_from() {
    let bytes = message_to_wire("", &RuntimeValue::Int(1)).unwrap();
    let (from, _) = message_from_wire(&bytes).unwrap();
    assert_eq!(from, "");
}

#[test]
fn message_wire_list_and_tuple_and_set_roundtrip() {
    let list = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(vec![
        RuntimeValue::Int(1),
        RuntimeValue::Text(Rc::new("two".to_string())),
        RuntimeValue::Bool(true),
    ]))));
    assert_wire_roundtrips(&list, "");

    let tup = RuntimeValue::Tuple(Rc::new(vec![RuntimeValue::Int(1), RuntimeValue::Char('q')]));
    assert_wire_roundtrips(&tup, "");

    let set = RuntimeValue::Set(Rc::new(RefCell::new(vec![
        RuntimeValue::Int(7),
        RuntimeValue::Int(8),
    ])));
    assert_wire_roundtrips(&set, "");
}

#[test]
fn message_wire_single_entry_map_roundtrips() {
    let mut m: MapStorage = MapStorage::default();
    m.insert(
        RuntimeValue::Text(Rc::new("k".to_string())),
        RuntimeValue::Int(9),
    );
    let v = RuntimeValue::Map(Rc::new(RefCell::new(m)));
    assert_wire_roundtrips(&v, "");
}

#[test]
fn message_wire_struct_roundtrips_by_field() {
    let mut fields = HashMap::new();
    fields.insert("x".to_string(), RuntimeValue::Int(1));
    fields.insert("y".to_string(), RuntimeValue::Int(2));
    let v = RuntimeValue::Struct(Box::new(StructValue {
        type_name: "Point".to_string(),
        fields,
    }));
    let bytes = message_to_wire("alice", &v).unwrap();
    let (_from, back) = message_from_wire(&bytes).unwrap();
    match back {
        RuntimeValue::Struct(s) => {
            assert_eq!(s.type_name, "Point");
            assert_eq!(s.fields.get("x"), Some(&RuntimeValue::Int(1)));
            assert_eq!(s.fields.get("y"), Some(&RuntimeValue::Int(2)));
        }
        other => panic!("expected a struct, got {other:?}"),
    }
}

#[test]
fn message_wire_inductive_roundtrips() {
    let v = RuntimeValue::Inductive(std::rc::Rc::new(InductiveValue {
        inductive_type: "Option".to_string(),
        constructor: "Some".to_string(),
        args: vec![RuntimeValue::Int(42)],
    }));
    assert_wire_roundtrips(&v, "");
}

#[test]
fn message_wire_nested_list_of_structs_roundtrips() {
    let mut fields = HashMap::new();
    fields.insert("n".to_string(), RuntimeValue::Int(1));
    let s = RuntimeValue::Struct(Box::new(StructValue {
        type_name: "Item".to_string(),
        fields,
    }));
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(vec![
        s,
        RuntimeValue::Tuple(Rc::new(vec![
            RuntimeValue::Int(3),
            RuntimeValue::Bool(false),
        ])),
    ]))));
    assert_wire_roundtrips(&v, "carol");
}

#[test]
fn message_wire_channel_handle_is_not_network_portable() {
    // A channel id is local to this process's scheduler — it cannot travel.
    let v = RuntimeValue::Chan(logicaffeine_runtime::ChanId(3));
    let err = message_to_wire("", &v).expect_err("a channel must not be sendable over the network");
    assert!(
        err.contains("channel") || err.contains("task"),
        "got: {err}"
    );
}

#[test]
fn message_wire_function_is_rejected_with_a_clear_error() {
    let v = RuntimeValue::Function(Box::new(ClosureValue {
        body_index: 0,
        captured_env: HashMap::default(),
        param_names: vec![],
        generated: None,
    }));
    let err = message_to_wire("", &v).expect_err("a closure must not be sendable");
    assert!(err.contains("Function"), "got: {err}");
}

#[test]
fn message_wire_malformed_bytes_decode_to_none() {
    assert!(message_from_wire(b"").is_none()); // empty
    assert!(message_from_wire(b"\xff\xff\xff\xff garbage").is_none()); // not a valid frame
                                                                       // A truncated-but-plausible prefix of a real message must not panic.
    let good = message_to_wire("alice", &RuntimeValue::Text(Rc::new("hi".to_string()))).unwrap();
    assert!(message_from_wire(&good[..good.len() / 2]).is_none());
}

// =========================================================================
// Codec hardening — fidelity, canonicality, integrity, speed, to the point
// of absurdity.
// =========================================================================

/// A round-trip is byte-stable: encode → decode → re-encode yields identical
/// bytes. This proves the value reconstructed EXACTLY (bit-for-bit — floats,
/// NaN, `-0.0` and all, since we compare bytes not `PartialEq`), that encoding
/// is deterministic, and that structs/maps are canonical (hash order can't
/// leak in). It's the workhorse assertion for the property fuzzer.
fn assert_wire_stable(v: &RuntimeValue) {
    let once = message_to_wire("peer", v).expect("encode");
    let (from, back) = message_from_wire(&once).expect("decode");
    assert_eq!(from, "peer", "sender lost");
    let twice = message_to_wire("peer", &back).expect("re-encode");
    assert_eq!(once, twice, "round-trip was not byte-stable for {v:?}");
}

#[test]
fn wire_boundary_ints_roundtrip() {
    for n in [
        0i64,
        1,
        -1,
        i64::MIN,
        i64::MAX,
        i64::MIN + 1,
        i64::MAX - 1,
        i32::MIN as i64,
        i32::MAX as i64,
    ] {
        assert_wire_stable(&RuntimeValue::Int(n));
    }
}

#[test]
fn wire_special_floats_roundtrip_bit_exact() {
    for f in [
        0.0f64,
        -0.0,
        1.0,
        -1.0,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
        f64::MIN,
        f64::MAX,
        f64::MIN_POSITIVE,
        1e-308,
        -1e308,
        std::f64::consts::PI,
    ] {
        let bytes = message_to_wire("", &RuntimeValue::Float(f)).unwrap();
        let (_, back) = message_from_wire(&bytes).unwrap();
        let RuntimeValue::Float(g) = back else {
            panic!("not a float")
        };
        assert_eq!(f.to_bits(), g.to_bits(), "float {f} lost bits");
    }
}

#[test]
fn wire_unicode_text_and_char_roundtrip() {
    for s in [
        "",
        "ascii",
        "héllo",
        "日本語",
        "emoji 😀🎉",
        "null\0byte",
        "tabs\tand\nnewlines",
    ] {
        assert_wire_stable(&RuntimeValue::Text(Rc::new(s.to_string())));
    }
    for c in ['a', '\0', '😀', '\u{10FFFF}', 'λ', '\n', '\u{1}'] {
        assert_wire_stable(&RuntimeValue::Char(c));
    }
}

#[test]
fn wire_empty_collections_roundtrip() {
    assert_wire_stable(&RuntimeValue::List(Rc::new(RefCell::new(
        ListRepr::from_values(vec![]),
    ))));
    assert_wire_stable(&RuntimeValue::Tuple(Rc::new(vec![])));
    assert_wire_stable(&RuntimeValue::Set(Rc::new(RefCell::new(vec![]))));
    assert_wire_stable(&RuntimeValue::Map(Rc::new(RefCell::new(
        MapStorage::default(),
    ))));
    assert_wire_stable(&RuntimeValue::Struct(Box::new(StructValue {
        type_name: "Empty".to_string(),
        fields: HashMap::new(),
    })));
    assert_wire_stable(&RuntimeValue::Inductive(std::rc::Rc::new(InductiveValue {
        inductive_type: "Unit".to_string(),
        constructor: "Unit".to_string(),
        args: vec![],
    })));
}

#[test]
fn wire_temporal_and_misc_scalars_roundtrip() {
    for v in [
        RuntimeValue::Nothing,
        RuntimeValue::Bool(true),
        RuntimeValue::Bool(false),
        RuntimeValue::Duration(i64::MAX),
        RuntimeValue::Date(i32::MIN),
        RuntimeValue::Moment(-1),
        RuntimeValue::Span {
            months: i32::MAX,
            days: i32::MIN,
        },
        RuntimeValue::Time(0),
        RuntimeValue::Peer(Rc::new("ws://host:9944".to_string())),
    ] {
        assert_wire_stable(&v);
    }
}

#[test]
fn wire_struct_field_order_is_canonical() {
    // The same fields in a different insertion order encode to the SAME bytes.
    let mut a = HashMap::new();
    a.insert("z".to_string(), RuntimeValue::Int(1));
    a.insert("a".to_string(), RuntimeValue::Int(2));
    a.insert("m".to_string(), RuntimeValue::Int(3));
    let mut b = HashMap::new();
    b.insert("m".to_string(), RuntimeValue::Int(3));
    b.insert("z".to_string(), RuntimeValue::Int(1));
    b.insert("a".to_string(), RuntimeValue::Int(2));
    let va = RuntimeValue::Struct(Box::new(StructValue {
        type_name: "S".into(),
        fields: a,
    }));
    let vb = RuntimeValue::Struct(Box::new(StructValue {
        type_name: "S".into(),
        fields: b,
    }));
    assert_eq!(
        message_to_wire("x", &va).unwrap(),
        message_to_wire("x", &vb).unwrap(),
        "struct encoding must be canonical (field order independent)"
    );
}

#[test]
fn wire_map_entry_order_is_canonical() {
    let mut a = MapStorage::default();
    a.insert(RuntimeValue::Int(3), RuntimeValue::Int(30));
    a.insert(RuntimeValue::Int(1), RuntimeValue::Int(10));
    a.insert(RuntimeValue::Int(2), RuntimeValue::Int(20));
    let mut b = MapStorage::default();
    b.insert(RuntimeValue::Int(1), RuntimeValue::Int(10));
    b.insert(RuntimeValue::Int(2), RuntimeValue::Int(20));
    b.insert(RuntimeValue::Int(3), RuntimeValue::Int(30));
    let va = RuntimeValue::Map(Rc::new(RefCell::new(a)));
    let vb = RuntimeValue::Map(Rc::new(RefCell::new(b)));
    assert_eq!(
        message_to_wire("x", &va).unwrap(),
        message_to_wire("x", &vb).unwrap(),
        "map encoding must be canonical (entry order independent)"
    );
}

#[test]
fn wire_rejects_nonportable_buried_in_a_container() {
    // A channel buried in a list — caught, with a clear error, never dropped.
    let in_list = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(vec![
        RuntimeValue::Int(1),
        RuntimeValue::Chan(logicaffeine_runtime::ChanId(5)),
    ]))));
    let err = message_to_wire("", &in_list).expect_err("buried channel must be rejected");
    assert!(
        err.contains("channel") || err.contains("task"),
        "got: {err}"
    );

    // A task handle buried in a map value.
    let mut m = MapStorage::default();
    m.insert(
        RuntimeValue::Int(1),
        RuntimeValue::TaskHandle(logicaffeine_runtime::TaskId(7)),
    );
    let in_map = RuntimeValue::Map(Rc::new(RefCell::new(m)));
    assert!(
        message_to_wire("", &in_map).is_err(),
        "buried task handle must be rejected"
    );

    // A closure buried in a struct field.
    let mut fields = HashMap::new();
    fields.insert(
        "f".to_string(),
        RuntimeValue::Function(Box::new(ClosureValue {
            body_index: 0,
            captured_env: HashMap::default(),
            param_names: vec![],
            generated: None,
        })),
    );
    let in_struct = RuntimeValue::Struct(Box::new(StructValue {
        type_name: "Holder".into(),
        fields,
    }));
    let err = message_to_wire("", &in_struct).expect_err("buried closure must be rejected");
    assert!(err.contains("Function"), "got: {err}");
}

#[test]
fn wire_checked_detects_corruption() {
    let v = RuntimeValue::Text(Rc::new("important".to_string()));
    let good = message_to_wire_with("a", &v, WireCodec::Native, WireIntegrity::Checked).unwrap();
    assert!(
        message_from_wire(&good).is_some(),
        "intact checked message decodes"
    );
    // Flip the final payload byte — the checksum must catch it.
    let mut bad = good.clone();
    *bad.last_mut().unwrap() ^= 0xFF;
    assert!(
        message_from_wire(&bad).is_none(),
        "corruption must be rejected"
    );
    // Flip a checksum byte too.
    let mut bad2 = good;
    bad2[1] ^= 0xFF;
    assert!(
        message_from_wire(&bad2).is_none(),
        "a mangled checksum must be rejected"
    );
}

#[test]
fn wire_raw_skips_the_checksum_for_speed() {
    let v = RuntimeValue::Int(7);
    let raw = message_to_wire_with("", &v, WireCodec::Native, WireIntegrity::Raw).unwrap();
    let checked = message_to_wire_with("", &v, WireCodec::Native, WireIntegrity::Checked).unwrap();
    assert_eq!(
        checked.len(),
        raw.len() + 8,
        "checked adds exactly the 8-byte checksum"
    );
    assert_eq!(message_from_wire(&raw).unwrap().1, RuntimeValue::Int(7));
    // A flipped byte in a RAW message is NOT caught (no integrity) — it just
    // decodes to whatever it decodes to, or fails to decode; either way no panic.
    let mut tampered = raw;
    *tampered.last_mut().unwrap() ^= 0x01;
    let _ = message_from_wire(&tampered);
}

#[test]
fn wire_all_codec_and_integrity_modes_interoperate() {
    let v = RuntimeValue::Text(Rc::new("hello".to_string()));
    for codec in [WireCodec::Native, WireCodec::Json] {
        for integrity in [WireIntegrity::Raw, WireIntegrity::Checked] {
            let bytes = message_to_wire_with("s", &v, codec, integrity).unwrap();
            let (from, back) = message_from_wire(&bytes).expect("decodes any mode");
            assert_eq!(from, "s");
            assert_eq!(back, v, "{codec:?}/{integrity:?} did not round-trip");
        }
    }
}

#[test]
fn wire_compression_shrinks_redundant_payloads_and_roundtrips() {
    // A big redundant payload (the same string 500×) compresses hard.
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
        (0..500)
            .map(|_| RuntimeValue::Text(Rc::new("the same repeated string".to_string())))
            .collect(),
    ))));
    let plain = message_to_wire("", &v).unwrap();
    let zipped = with_compression(|| message_to_wire("", &v).unwrap());
    assert!(
        zipped.len() * 2 < plain.len(),
        "redundant data should compress: {} vs {}",
        zipped.len(),
        plain.len()
    );
    // …and inflates transparently on the way back in.
    let count = |b: &[u8]| match message_from_wire(b).unwrap().1 {
        RuntimeValue::List(l) => l.borrow().len(),
        other => panic!("expected a list, got {other:?}"),
    };
    assert_eq!(count(&zipped), 500);
    assert_eq!(count(&plain), 500);
}

#[test]
fn wire_compression_never_grows_a_message() {
    // A tiny / already-compact value: compression wouldn't help, so it's shipped
    // RAW ("keep only if it shrank") — never bigger, never a panic.
    let v = RuntimeValue::Int(42);
    let plain = message_to_wire("", &v).unwrap();
    let maybe = with_compression(|| message_to_wire("", &v).unwrap());
    assert!(
        maybe.len() <= plain.len(),
        "compression must never grow a message"
    );
    assert_eq!(message_from_wire(&maybe).unwrap().1, RuntimeValue::Int(42));
}

#[test]
fn wire_compressed_message_integrity_is_checked_before_inflate() {
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
        (0..500)
            .map(|_| RuntimeValue::Text(Rc::new("redundant".to_string())))
            .collect(),
    ))));
    let mut zipped = with_compression(|| {
        message_to_wire_with("", &v, WireCodec::Native, WireIntegrity::Checked)
    })
    .unwrap();
    assert!(
        message_from_wire(&zipped).is_some(),
        "intact compressed message decodes"
    );
    // Corrupt a compressed byte: the checksum (over the compressed bytes) rejects
    // it BEFORE we ever try to inflate — a clean None, no decompressor panic.
    *zipped.last_mut().unwrap() ^= 0xFF;
    assert!(
        message_from_wire(&zipped).is_none(),
        "corruption of a compressed message must be rejected"
    );
}

fn redundant_list(n: usize) -> RuntimeValue {
    RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
        (0..n)
            .map(|_| RuntimeValue::Text(Rc::new("the same repeated string value".to_string())))
            .collect(),
    ))))
}
fn count_list(v: &RuntimeValue) -> usize {
    match v {
        RuntimeValue::List(l) => l.borrow().len(),
        other => panic!("expected a list, got {other:?}"),
    }
}

#[test]
fn wire_lz4_roundtrips_and_shrinks_redundant() {
    let v = redundant_list(500);
    let plain = message_to_wire("", &v).unwrap();
    let lz = with_compression_codec(WireCompression::Lz4, || message_to_wire("", &v).unwrap());
    assert!(
        lz.len() < plain.len(),
        "lz4 should shrink redundant data: {} vs {}",
        lz.len(),
        plain.len()
    );
    // Self-describing: the header records the codec, so decode auto-detects it.
    assert_eq!(count_list(&message_from_wire(&lz).unwrap().1), 500);
}

#[test]
fn wire_compression_codec_is_self_describing() {
    // Deflate + lz4 ship everywhere (pure-Rust). Each is auto-detected on decode
    // with no out-of-band hint — the header byte carries the codec id.
    let v = redundant_list(300);
    for c in [WireCompression::Deflate, WireCompression::Lz4] {
        let bytes = with_compression_codec(c, || message_to_wire("", &v).unwrap());
        assert_eq!(
            count_list(&message_from_wire(&bytes).unwrap().1),
            300,
            "codec {c:?} self-describes"
        );
        let (_, comp, _) = unframe(&bytes).unwrap();
        assert_eq!(comp, c, "header round-trips the codec id for {c:?}");
    }
}

#[test]
fn wire_old_deflate_bytes_still_decode() {
    // Back-compat: a message framed the OLD way (H_COMPRESSED set, codec bits 0 =
    // deflate) must still decode after the 2-bit codec field was added.
    let v = redundant_list(200);
    let body = {
        let mut out = Vec::new();
        write_str("", &mut out);
        native_encode(&v, &mut out).unwrap();
        miniz_oxide::deflate::compress_to_vec(&out, 6)
    };
    let mut legacy = vec![H_COMPRESSED]; // legacy header: no codec bits
    legacy.extend_from_slice(&body);
    assert_eq!(count_list(&message_from_wire(&legacy).unwrap().1), 200);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn wire_zstd_roundtrips_native_and_is_self_describing() {
    let v = redundant_list(500);
    let plain = message_to_wire("", &v).unwrap();
    let z = with_compression_codec(WireCompression::Zstd, || message_to_wire("", &v).unwrap());
    assert!(
        z.len() < plain.len(),
        "zstd should shrink redundant data: {} vs {}",
        z.len(),
        plain.len()
    );
    assert_eq!(count_list(&message_from_wire(&z).unwrap().1), 500);
    let (_, comp, _) = unframe(&z).unwrap();
    assert_eq!(comp, WireCompression::Zstd, "header records zstd");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn wire_zstd_decodes_via_ruzstd_parity() {
    // The native C zstd encoder writes a standard frame; the pure-Rust ruzstd
    // decoder (the browser's decode path) must inflate it byte-identically — so a
    // native-sent zstd message is decodable by a wasm peer.
    let v = redundant_list(500);
    let z = with_compression_codec(WireCompression::Zstd, || message_to_wire("", &v).unwrap());
    let (_codec, comp, body) = unframe(&z).expect("unframe");
    assert_eq!(comp, WireCompression::Zstd);
    let via_c = zstd::decode_all(body).expect("C zstd decode");
    let via_ruzstd = zstd_decode_ruzstd(body).expect("ruzstd decode");
    assert_eq!(via_c, via_ruzstd, "ruzstd must match C zstd byte-for-byte");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn wire_compression_level_dial_trades_size_and_roundtrips() {
    // A moderately-compressible payload so the effort level actually moves the
    // size. Max ≤ Balanced ≤ Fast in bytes; every level decodes exactly.
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
        (0..2000)
            .map(|i| RuntimeValue::Text(Rc::new(format!("event-{i}-status-{}", i % 37))))
            .collect(),
    ))));
    let at = |lvl| {
        with_compression_level(lvl, || {
            with_compression_codec(WireCompression::Zstd, || message_to_wire("", &v).unwrap())
        })
    };
    let fast = at(WireCompressionLevel::Fast);
    let bal = at(WireCompressionLevel::Balanced);
    let max = at(WireCompressionLevel::Max);
    assert!(
        max.len() <= bal.len() && bal.len() <= fast.len(),
        "max {} ≤ bal {} ≤ fast {}",
        max.len(),
        bal.len(),
        fast.len()
    );
    for b in [&fast, &bal, &max] {
        assert!(message_from_wire(b).is_some(), "every level decodes");
    }
    // The level is a sender-only preference — it never leaks past the scope.
    let default =
        with_compression_codec(WireCompression::Zstd, || message_to_wire("", &v).unwrap());
    assert_eq!(default.len(), bal.len(), "default level is Balanced");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn wire_zstd_ratio_beats_deflate_on_redundant() {
    let v = redundant_list(1000);
    let d = with_compression_codec(WireCompression::Deflate, || {
        message_to_wire("", &v).unwrap()
    });
    let z = with_compression_codec(WireCompression::Zstd, || message_to_wire("", &v).unwrap());
    assert!(
        z.len() <= d.len(),
        "zstd should match or beat deflate: zstd {} vs deflate {}",
        z.len(),
        d.len()
    );
}

fn point(x: i64, y: i64) -> RuntimeValue {
    let mut f = HashMap::new();
    f.insert("x".to_string(), RuntimeValue::Int(x));
    f.insert("y".to_string(), RuntimeValue::Int(y));
    RuntimeValue::Struct(Box::new(StructValue {
        type_name: "Point".to_string(),
        fields: f,
    }))
}
fn struct_list(n: i64) -> RuntimeValue {
    RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
        (0..n).map(|i| point(i, i * 2)).collect(),
    ))))
}
fn enum_val(ty: &str, ctor: &str, args: Vec<RuntimeValue>) -> RuntimeValue {
    RuntimeValue::Inductive(std::rc::Rc::new(InductiveValue {
        inductive_type: ty.to_string(),
        constructor: ctor.to_string(),
        args,
    }))
}

/// Encode `v` (asserting its top tag), decode it, then re-encode the decoded
/// value and assert byte-equality. Encoding is canonical (fields sorted), so this
/// proves "decodes to the exact rows" deterministically — unlike comparing
/// `materialize` output, whose struct field order is per-HashMap random.
fn assert_columnar_roundtrip(v: &RuntimeValue, expect_tag: u8) {
    let mut buf = Vec::new();
    native_encode(v, &mut buf).unwrap();
    assert_eq!(buf[0], expect_tag, "top wire tag");
    let mut pos = 0;
    let back = native_decode(&buf, &mut pos).unwrap();
    assert_eq!(pos, buf.len(), "decode consumes the whole buffer");
    let mut buf2 = Vec::new();
    native_encode(&back, &mut buf2).unwrap();
    assert_eq!(
        buf2, buf,
        "re-encode of the decoded value is byte-identical (exact rows, canonical)"
    );
}

#[test]
fn wire_homogeneous_struct_list_packs_columnar() {
    assert_columnar_roundtrip(&struct_list(1000), T_STRUCTS);
}

#[test]
fn wire_columnar_struct_list_is_far_smaller_than_boxed() {
    // The old boxed path re-emitted "Point"/"x"/"y" strings on EVERY row
    // (~16 B/row ⇒ ~16 KB for 1000). Columnar writes the schema once + two packed
    // int columns ⇒ a few KB. A threshold cleanly between the two regimes.
    let mut buf = Vec::new();
    native_encode(&struct_list(1000), &mut buf).unwrap();
    assert!(
        buf.len() < 6000,
        "columnar 1000×Point should be well under boxed's ~16 KB, was {}",
        buf.len()
    );
}

#[test]
fn wire_columnar_int_field_is_memcpy_under_fixed() {
    // Columnar fields honor the numeric dial: under Fixed, each int column is raw
    // 8-byte i64 (memcpy) — wider than varint. Boxed structs ignored the dial, so
    // this size difference is exactly what proves the fields pack as columns.
    let v = struct_list(200);
    let varint = {
        let mut b = Vec::new();
        native_encode(&v, &mut b).unwrap();
        b
    };
    let fixed = {
        let mut b = Vec::new();
        with_fixed_numerics(|| native_encode(&v, &mut b).unwrap());
        b
    };
    assert_eq!(varint[0], T_STRUCTS);
    assert_eq!(fixed[0], T_STRUCTS);
    assert!(
        fixed.len() > varint.len(),
        "fixed int columns are memcpy-wide: fixed {} vs varint {}",
        fixed.len(),
        varint.len()
    );
    // The fixed encoding decodes and re-encodes (under fixed) byte-identically.
    let mut pos = 0;
    let back = native_decode(&fixed, &mut pos).unwrap();
    let mut re = Vec::new();
    with_fixed_numerics(|| native_encode(&back, &mut re).unwrap());
    assert_eq!(re, fixed, "fixed columnar decodes to the exact rows");
}

#[test]
fn wire_ragged_struct_list_falls_back_to_boxed() {
    // Different field sets ⇒ not homogeneous ⇒ the generic per-element path.
    let a = point(1, 2);
    let mut bf = HashMap::new();
    bf.insert("x".to_string(), RuntimeValue::Int(3)); // no "y"
    let b = RuntimeValue::Struct(Box::new(StructValue {
        type_name: "Point".to_string(),
        fields: bf,
    }));
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(vec![a, b]))));
    assert_columnar_roundtrip(&v, T_LIST); // ragged struct list stays boxed
}

#[test]
fn wire_columnar_struct_roundtrip_is_byte_stable() {
    assert_wire_stable(&struct_list(50));
}

#[test]
fn wire_in_memory_columnar_structs_encode_identically_to_boxed() {
    // A `from_values`-built Structs repr (in-memory columns) and a hand-built
    // Boxed struct list with the same rows must encode to byte-identical wire —
    // the in-memory columns stream out exactly as the boxed path would gather them.
    let rows: Vec<RuntimeValue> = (0..100).map(|i| point(i, i * 2)).collect();
    let repr = ListRepr::from_values(rows.clone());
    assert!(
        matches!(repr, ListRepr::Structs { .. }),
        "from_values de-boxes a struct list to columns"
    );
    let columnar = RuntimeValue::List(Rc::new(RefCell::new(repr)));
    let boxed = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(rows))));
    assert_eq!(
        message_to_wire("p", &columnar).unwrap(),
        message_to_wire("p", &boxed).unwrap(),
        "in-memory columns encode byte-identically to the boxed columnar path"
    );
}

#[test]
fn wire_struct_list_decodes_to_columnar_repr() {
    // A received struct list decodes DIRECTLY into the columnar in-memory repr —
    // no per-row `StructValue` rebuild (the receive-throughput win).
    let bytes = message_to_wire("p", &struct_list(100)).unwrap();
    match message_from_wire(&bytes).unwrap().1 {
        RuntimeValue::List(l) => {
            assert!(
                matches!(&*l.borrow(), ListRepr::Structs { .. }),
                "decodes to the columnar Structs repr"
            );
            assert_eq!(l.borrow().len(), 100);
        }
        other => panic!("expected a list, got {other:?}"),
    }
}

#[test]
fn wire_nullary_enum_list_packs_dictionary() {
    let names = ["Red", "Green", "Blue"];
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
        (0..900)
            .map(|i| enum_val("Color", names[i % 3], vec![]))
            .collect(),
    ))));
    let mut buf = Vec::new();
    native_encode(&v, &mut buf).unwrap();
    assert!(
        buf.len() < 1200,
        "dictionaried enum list should be tiny, was {}",
        buf.len()
    );
    assert_columnar_roundtrip(&v, T_INDUCTIVES);
}

#[test]
fn wire_arg_enum_list_packs_columnar() {
    // Arg-bearing enums now pack as a tagged union (T_INDUCTIVES) — a constructor
    // dictionary + index column + dense per-constructor arg columns — not boxed.
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
        (0..10)
            .map(|i| enum_val("Option", "Some", vec![RuntimeValue::Int(i)]))
            .collect(),
    ))));
    assert_columnar_roundtrip(&v, T_INDUCTIVES);
}

#[test]
fn wire_mixed_arity_enum_list_packs_columnar() {
    // Some(1), None, Some(2), None — mixed constructors/arities round-trip exact.
    let rows: Vec<RuntimeValue> = (0..20)
        .map(|i| {
            if i % 2 == 0 {
                enum_val("Option", "Some", vec![RuntimeValue::Int(i)])
            } else {
                enum_val("Option", "None", vec![])
            }
        })
        .collect();
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(rows))));
    assert_columnar_roundtrip(&v, T_INDUCTIVES);
}

#[test]
fn wire_enum_list_decodes_to_columnar_repr() {
    // A received enum list decodes DIRECTLY into the columnar Inductives repr.
    let rows: Vec<RuntimeValue> = (0..50)
        .map(|i| enum_val("Option", "Some", vec![RuntimeValue::Int(i)]))
        .collect();
    let bytes = message_to_wire(
        "p",
        &RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(rows)))),
    )
    .unwrap();
    match message_from_wire(&bytes).unwrap().1 {
        RuntimeValue::List(l) => {
            assert!(
                matches!(&*l.borrow(), ListRepr::Inductives { .. }),
                "decodes to the columnar Inductives repr"
            );
            assert_eq!(l.borrow().len(), 50);
        }
        other => panic!("expected a list, got {other:?}"),
    }
}

#[test]
fn wire_schema_dictionary_sends_schema_once_and_shrinks() {
    // With a connection-scoped cache, a struct schema (type + field names) is sent
    // ONCE; later messages of the same shape reference it by id and are smaller.
    let v = struct_list(50);
    let mut send_cache = WireSchemaCache::default();
    let mut recv_cache = WireSchemaCache::default();
    let m1 = message_to_wire_cached(
        "p",
        &v,
        WireCodec::Native,
        WireIntegrity::Raw,
        &mut send_cache,
    )
    .unwrap();
    let m2 = message_to_wire_cached(
        "p",
        &v,
        WireCodec::Native,
        WireIntegrity::Raw,
        &mut send_cache,
    )
    .unwrap();
    assert!(
        m2.len() < m1.len(),
        "the 2nd message references the schema and is smaller: {} vs {}",
        m2.len(),
        m1.len()
    );
    // The receiver decodes both in order with its own cache; each reconstructs the
    // exact rows (proven by canonical stateless re-encode equality).
    let d1 = message_from_wire_cached(&m1, &mut recv_cache).unwrap().1;
    let d2 = message_from_wire_cached(&m2, &mut recv_cache).unwrap().1;
    let canon = |x: &RuntimeValue| message_to_wire("p", x).unwrap();
    assert_eq!(canon(&d1), canon(&v));
    assert_eq!(canon(&d2), canon(&v));
}

#[test]
fn wire_single_struct_schema_ref_drops_field_names_and_shrinks() {
    // G4: a LONE struct (not a list) under a connection cache sends its schema
    // once; the next same-shaped struct ships values only — no inline "x"/"y" —
    // so it is strictly smaller, and both still decode to the exact struct.
    let v = point(1, 2);
    let mut sc = WireSchemaCache::default();
    let mut rc = WireSchemaCache::default();
    let m1 =
        message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut sc).unwrap();
    let m2 =
        message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut sc).unwrap();
    assert!(
        m2.len() < m1.len(),
        "2nd lone struct references the schema and is smaller: {} vs {}",
        m2.len(),
        m1.len()
    );
    let canon = |x: &RuntimeValue| message_to_wire("p", x).unwrap();
    let d1 = message_from_wire_cached(&m1, &mut rc).unwrap().1;
    let d2 = message_from_wire_cached(&m2, &mut rc).unwrap().1;
    assert_eq!(canon(&d1), canon(&v));
    assert_eq!(canon(&d2), canon(&v));
}

#[test]
fn wire_single_struct_sequential_ref_beats_inline_field_names() {
    // The sequential (1-byte id) ref is strictly smaller than the uncached inline
    // struct, because it replaces the type-name + every field-name string with one
    // id — the clean, uncompressed size win that closes the postcard gap.
    let v = point(7, 9);
    let inline = message_to_wire("p", &v).unwrap();
    let mut sc = WireSchemaCache::sequential();
    let _def =
        message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut sc).unwrap();
    let refmsg =
        message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut sc).unwrap();
    assert!(
        refmsg.len() < inline.len(),
        "schema-ref ({}) must beat inline field-names ({})",
        refmsg.len(),
        inline.len()
    );
}

#[test]
fn wire_single_struct_def_decodes_without_a_cache() {
    // The FIRST cached lone-struct message carries its schema inline (a "def"), so
    // a stateless decoder still reconstructs it.
    let v = point(3, 4);
    let mut sc = WireSchemaCache::default();
    let m1 =
        message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut sc).unwrap();
    let d = message_from_wire(&m1).unwrap().1;
    assert_eq!(
        message_to_wire("p", &d).unwrap(),
        message_to_wire("p", &v).unwrap()
    );
}

#[test]
fn wire_single_struct_ref_without_cache_fails_cleanly() {
    // A schema-reference lone struct decoded WITHOUT the cache that defined it
    // returns None (clean) — never a mis-decode, never a panic.
    let v = point(5, 6);
    let mut sc = WireSchemaCache::default();
    let _m1 =
        message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut sc).unwrap();
    let m2 =
        message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut sc).unwrap();
    assert!(
        message_from_wire(&m2).is_none(),
        "a bare schema-ref must not decode without its cache"
    );
}

#[test]
fn wire_single_struct_uncached_stays_inline_tag() {
    // No cache → the lone struct still uses the self-describing T_STRUCT tag (field
    // names inline), keeping every non-cached encode byte-identical to before G4.
    let v = point(1, 2);
    let bytes = message_to_wire("p", &v).unwrap();
    let (codec, _comp, body) = unframe(&bytes).unwrap();
    assert!(matches!(codec, WireCodec::Native));
    let mut pos = 0;
    skip_str(body, &mut pos).unwrap();
    assert_eq!(
        body[pos], T_STRUCT,
        "uncached lone struct must stay the inline T_STRUCT form"
    );
}

// ---- Pillar B: type-id substrate (default-on name elision, beats raw varint) ----

#[test]
fn wire_struct_type_id_elides_names_and_beats_inline() {
    // With a shared type registry (both ends derive it from their program's type
    // defs), a struct ships its small registry id instead of its type/field NAMES —
    // strictly smaller than the self-describing inline form, on the FIRST message.
    let v = point(1, 2);
    let schemas = vec![("Point".to_string(), vec!["x".to_string(), "y".to_string()])];
    let with_reg = with_type_registry(WireTypeRegistry::new(schemas.clone()), || {
        message_to_wire("p", &v).unwrap()
    });
    let inline = message_to_wire("p", &v).unwrap();
    assert!(
        with_reg.len() < inline.len(),
        "type-id encode ({}) must elide names vs inline ({})",
        with_reg.len(),
        inline.len()
    );
    // Decoding with the same registry reconstructs the exact struct.
    let back = with_type_registry(WireTypeRegistry::new(schemas), || {
        message_from_wire(&with_reg).unwrap().1
    });
    assert_eq!(
        message_to_wire("p", &back).unwrap(),
        inline,
        "type-id round-trips to the exact value"
    );
}

#[test]
fn wire_struct_type_id_falls_back_to_inline_for_unknown_type() {
    // A registry that does NOT contain the struct's type → byte-identical inline form
    // (so a cross-version / non-Logos peer is never confused).
    let v = point(1, 2);
    let other = vec![("Other".to_string(), vec!["a".to_string()])];
    let bytes = with_type_registry(WireTypeRegistry::new(other), || {
        message_to_wire("p", &v).unwrap()
    });
    assert_eq!(
        bytes,
        message_to_wire("p", &v).unwrap(),
        "unknown type falls back to byte-identical inline"
    );
}

#[test]
fn wire_struct_type_id_unknown_id_fails_cleanly() {
    // Bytes encoded against one registry, decoded against an EMPTY one: the id can't
    // resolve → None (clean), never a mis-decode.
    let v = point(1, 2);
    let schemas = vec![("Point".to_string(), vec!["x".to_string(), "y".to_string()])];
    let bytes = with_type_registry(WireTypeRegistry::new(schemas), || {
        message_to_wire("p", &v).unwrap()
    });
    let decoded = with_type_registry(WireTypeRegistry::new(vec![]), || message_from_wire(&bytes));
    assert!(
        decoded.is_none(),
        "an unresolvable type-id must fail cleanly, not mis-decode"
    );
}

// ---- Pillar D: the no-brainer auto-tuner (`best`) ----

#[test]
fn wire_best_smallest_is_never_larger_than_any_single_knob() {
    // The auto-tuner's contract: `best(Smallest)` ≤ EVERY single-dial encoding, on EVERY
    // workload — and it round-trips with no decode hint (every form is self-describing).
    let il = |v: Vec<i64>| RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(v))));
    let fl = |v: Vec<f64>| RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(v))));
    let workloads: Vec<(&str, RuntimeValue)> = vec![
        ("sequential", il((0..256).collect())),
        (
            "random",
            il((0..256)
                .map(|i: i64| i.wrapping_mul(2_654_435_761))
                .collect()),
        ),
        ("repetitive", il(vec![7i64; 256])),
        ("clustered", il((0..256).map(|i| 1000 + (i % 8)).collect())),
        (
            "timeseries",
            fl((0..256).map(|i| 100.0 + i as f64 * 0.5).collect()),
        ),
        (
            "structs",
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(
                (0..40).map(|i| point(i, i * 2)).collect(),
            )))),
        ),
        ("int map", {
            let mut m = MapStorage::default();
            for k in 0..128i64 {
                m.insert(RuntimeValue::Int(k), RuntimeValue::Int(k * k));
            }
            RuntimeValue::Map(Rc::new(RefCell::new(m)))
        }),
        ("id→row map", {
            let mut m = MapStorage::default();
            for k in 0..40i64 {
                m.insert(RuntimeValue::Int(k), point(k, k * 2));
            }
            RuntimeValue::Map(Rc::new(RefCell::new(m)))
        }),
    ];
    for (name, v) in &workloads {
        let best = message_to_wire_best("p", v, WireGoal::Smallest).unwrap();
        for num in [
            WireNumerics::Varint,
            WireNumerics::Fixed,
            WireNumerics::GroupVarint,
        ] {
            let s = with_numerics(num, || message_to_wire("p", v)).unwrap();
            assert!(
                best.len() <= s.len(),
                "[{name}] best {} > numerics {:?} {}",
                best.len(),
                num,
                s.len()
            );
        }
        for st in [
            WireStructure::Off,
            WireStructure::Affine,
            WireStructure::Auto,
        ] {
            let s = with_structure(st, || message_to_wire("p", v)).unwrap();
            assert!(
                best.len() <= s.len(),
                "[{name}] best {} > structure {:?} {}",
                best.len(),
                st,
                s.len()
            );
        }
        for comp in [
            WireCompression::None,
            WireCompression::Deflate,
            WireCompression::Lz4,
            WireCompression::Zstd,
        ] {
            let s = with_compression_codec(comp, || message_to_wire("p", v)).unwrap();
            assert!(
                best.len() <= s.len(),
                "[{name}] best {} > compression {:?} {}",
                best.len(),
                comp,
                s.len()
            );
        }
        for f in [WireFloats::Memcpy, WireFloats::XorDelta] {
            let s = with_floats(f, || message_to_wire("p", v)).unwrap();
            assert!(
                best.len() <= s.len(),
                "[{name}] best {} > floats {:?} {}",
                best.len(),
                f,
                s.len()
            );
        }
        // Round-trips: decoding `best` then re-encoding under the default dials yields the
        // exact default encoding of the original value.
        let back = message_from_wire(&best).unwrap().1;
        assert_eq!(
            message_to_wire("p", &back).unwrap(),
            message_to_wire("p", v).unwrap(),
            "[{name}] best must round-trip to the exact value"
        );
    }
}

#[test]
fn wire_best_fastest_is_the_fixed_memcpy_form_and_round_trips() {
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints((0..100).collect()))));
    let fast = message_to_wire_best("p", &v, WireGoal::Fastest).unwrap();
    let fixed = with_numerics(WireNumerics::Fixed, || message_to_wire("p", &v)).unwrap();
    assert_eq!(fast, fixed, "Fastest must be the fixed memcpy-decode form");
    let back = message_from_wire(&fast).unwrap().1;
    assert_eq!(
        message_to_wire("p", &back).unwrap(),
        message_to_wire("p", &v).unwrap(),
        "Fastest must round-trip to the exact value"
    );
}

#[test]
fn wire_auto_is_the_no_brainer_default_searching_only_when_it_pays() {
    // `auto` is the "just send it" entry point. On a tiny message it ships the plain default (no
    // search overhead); on a bulk payload it runs the full Smallest bake-off. It is NEVER larger
    // than the default and always round-trips.
    let scalar = RuntimeValue::Int(42);
    assert_eq!(
        message_to_wire_auto("p", &scalar).unwrap(),
        message_to_wire("p", &scalar).unwrap(),
        "a tiny message skips the search and ships the plain default"
    );

    let bulk = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints((0..256).collect()))));
    let auto_bulk = message_to_wire_auto("p", &bulk).unwrap();
    assert_eq!(
        auto_bulk,
        message_to_wire_best("p", &bulk, WireGoal::Smallest).unwrap(),
        "a bulk payload gets the full Smallest search"
    );
    assert!(
        auto_bulk.len() < message_to_wire("p", &bulk).unwrap().len(),
        "and the search actually shrinks it below the default"
    );

    // Universal safety: never larger than the default, always round-trips — on every shape.
    let mut big_map = MapStorage::default();
    for k in 0..200i64 {
        big_map.insert(RuntimeValue::Int(k), RuntimeValue::Int(k * k));
    }
    for v in [
        scalar,
        bulk,
        RuntimeValue::Map(Rc::new(RefCell::new(big_map))),
    ] {
        let a = message_to_wire_auto("p", &v).unwrap();
        assert!(
            a.len() <= message_to_wire("p", &v).unwrap().len(),
            "auto is never larger than the default"
        );
        let back = message_from_wire(&a).unwrap().1;
        assert_eq!(
            message_to_wire("p", &back).unwrap(),
            message_to_wire("p", &v).unwrap(),
            "auto round-trips to the exact value"
        );
    }
}

#[test]
fn receiver_limits_refuse_a_too_deeply_nested_message() {
    // A 30-deep nested value encodes fine (well under MAX_ENCODE_DEPTH) and decodes by default, but
    // a receiver that declares a shallower max_depth REFUSES it — admission control enforced during
    // decode, before the recursion happens.
    let mut v = RuntimeValue::Nothing;
    for _ in 0..30 {
        v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(vec![v]))));
    }
    let bytes = message_to_wire("p", &v).unwrap();
    assert!(
        message_from_wire(&bytes).is_some(),
        "default limits accept a 30-deep message"
    );
    let tight = ReceiveLimits {
        max_depth: 10,
        ..Default::default()
    };
    assert!(
        with_receive_limits(tight, || message_from_wire(&bytes)).is_none(),
        "a receiver with max_depth=10 must refuse a 30-deep message"
    );
}

#[test]
fn receiver_survives_a_crafted_pathologically_deep_message_no_stack_overflow() {
    // THE DoS: an attacker crafts a message that is tiny in bytes (2 per level) but THOUSANDS of
    // containers deep. Without a decode-depth bound this recurses until the receiver's stack
    // overflows — a remote crash. The guard must REFUSE it cleanly, never overflow. (Hand-built raw
    // frame: 0x00 header = Native/uncompressed/raw, then from="" , then N×[T_LIST, count=1], then a
    // terminal — bypassing the encoder's own depth cap exactly as a hostile peer would.)
    let mut bytes = vec![0x00u8, 0x00u8]; // frame header + from=""
    for _ in 0..8000 {
        bytes.push(T_LIST);
        bytes.push(0x01); // uvarint count = 1
    }
    bytes.push(T_NOTHING);
    // A tight depth budget makes the guard bail far below any stack pressure — proving the
    // MECHANISM stops the recursion (the conservative default `max_depth` carries the same
    // protection with margin to spare in production release builds).
    let limits = ReceiveLimits {
        max_depth: 16,
        ..Default::default()
    };
    assert!(
        with_receive_limits(limits, || message_from_wire(&bytes)).is_none(),
        "an 8000-deep crafted message must be refused, not crash the receiver"
    );
}

#[test]
fn receiver_refuses_a_generator_bomb_tiny_descriptor_huge_array() {
    // The small-message-huge-output DoS that a byte budget CANNOT see: an affine column descriptor
    // `base,stride,count` is ~12 bytes but materializes `count` ints — a crafted billion-count is a
    // handful of bytes yet allocates gigabytes. `max_elements` gates the count before materializing.
    let affine = |count: u64| {
        let mut b = vec![0x00u8, 0x00u8, T_INTS_AFFINE]; // frame + from="" + tag
        write_uvarint(zigzag(0), &mut b); // base 0
        write_uvarint(zigzag(1), &mut b); // stride 1
        write_uvarint(count, &mut b);
        b
    };
    assert!(
        message_from_wire(&affine(1_000_000_000)).is_none(),
        "a 1e9-count affine descriptor (~12 bytes) must be refused, never allocated"
    );
    // A within-budget descriptor still decodes to exactly that many ints (the math-hack still works).
    match message_from_wire(&affine(100)).unwrap().1 {
        RuntimeValue::List(l) => assert_eq!(
            l.borrow().to_values().len(),
            100,
            "affine still expands within budget"
        ),
        other => panic!("expected a List, got {other:?}"),
    }
    // A tighter max_elements refuses a moderate count the default would allow, and admits a small one.
    let tight = ReceiveLimits {
        max_elements: 1000,
        ..Default::default()
    };
    assert!(
        with_receive_limits(tight, || message_from_wire(&affine(100_000))).is_none(),
        "max_elements=1000 refuses 100k"
    );
    assert!(
        with_receive_limits(tight, || message_from_wire(&affine(500))).is_some(),
        "max_elements=1000 admits 500"
    );
}

#[test]
fn receiver_refuses_an_oversized_message_before_decoding() {
    // A well-formed but large message is refused at the door when it exceeds the receiver's byte
    // budget — the gross-size admission gate, checked before any decompress / decode work.
    let big = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints((0..5000).collect()))));
    let bytes = message_to_wire("p", &big).unwrap();
    assert!(
        message_from_wire(&bytes).is_some(),
        "default byte budget accepts it"
    );
    let tight = ReceiveLimits {
        max_bytes: bytes.len() - 1,
        ..Default::default()
    };
    assert!(
        with_receive_limits(tight, || message_from_wire(&bytes)).is_none(),
        "a message one byte over max_bytes is refused before decode"
    );
}

#[test]
fn peer_profile_round_trips_and_falls_back_on_unknown() {
    let p = PeerProfile {
        limits: ReceiveLimits {
            max_bytes: 1 << 20,
            max_depth: 16,
            max_elements: 10_000,
            max_string_bytes: 4096,
            accept_computed: false,
        },
        registry_epoch: 0xDEAD_BEEF,
        features: FEAT_ZSTD | FEAT_TYPE_ID,
    };
    assert_eq!(
        decode_peer_profile(&encode_peer_profile(&p)),
        Some(p),
        "profile round-trips exactly"
    );
    assert_eq!(
        decode_peer_profile(&encode_peer_profile(&PeerProfile::default())),
        Some(PeerProfile::default()),
        "the default profile round-trips"
    );
    // An unknown version is recognized and not mis-parsed.
    let mut bad = encode_peer_profile(&p);
    bad[0] = 99;
    assert!(
        decode_peer_profile(&bad).is_none(),
        "an unknown profile version → None (caller uses defaults)"
    );
    // Truncation at any offset never panics or mis-decodes.
    let full = encode_peer_profile(&p);
    for cut in 0..full.len() {
        let _ = decode_peer_profile(&full[..cut]);
    }
    assert!(decode_peer_profile(&[]).is_none());
}

#[test]
fn negotiate_restricts_the_sender_to_the_receivers_exposed_surface() {
    // Both peers share an epoch and speak type-id → names elided; receiver accepts computed + both
    // speak it → may ship code; strongest shared compression chosen; the receiver's budget surfaces.
    let me = PeerProfile {
        registry_epoch: 7,
        features: FEAT_ZSTD | FEAT_LZ4 | FEAT_TYPE_ID | FEAT_COMPUTED,
        ..Default::default()
    };
    let peer = PeerProfile {
        limits: ReceiveLimits {
            max_bytes: 4096,
            ..Default::default()
        },
        registry_epoch: 7,
        features: FEAT_LZ4 | FEAT_TYPE_ID | FEAT_COMPUTED,
    };
    let n = negotiate(&me, &peer);
    assert!(
        n.use_type_id,
        "matching epochs + both type-id → elide names"
    );
    assert!(
        n.may_send_computed,
        "receiver accepts computed + both speak it → may ship a computation"
    );
    assert_eq!(
        n.compression,
        WireCompression::Lz4,
        "strongest compression BOTH understand (peer lacks zstd)"
    );
    assert_eq!(
        n.peer_max_bytes, 4096,
        "the receiver's byte budget surfaces to the sender"
    );

    // A receiver that declines code, has a different epoch, and shares no compression: the sender
    // backs OFF on every axis — exposed surface respected.
    let strict = PeerProfile {
        limits: ReceiveLimits {
            accept_computed: false,
            ..Default::default()
        },
        registry_epoch: 999,
        features: FEAT_TYPE_ID, // no compression, no computed
    };
    let n2 = negotiate(&me, &strict);
    assert!(!n2.use_type_id, "different epochs → names must travel");
    assert!(
        !n2.may_send_computed,
        "receiver declines code → never ship a computation"
    );
    assert_eq!(
        n2.compression,
        WireCompression::None,
        "no shared compression → send uncompressed"
    );
}

#[test]
fn negotiated_send_uses_all_knobs_within_the_receivers_surface() {
    let from = "p";
    let bulk = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints((0..256).collect()))));
    let default = message_to_wire(from, &bulk).unwrap();
    // Round-trip proof by byte-stability (container `PartialEq` is shallow): decode, re-encode under
    // the default dials, compare to the default encoding of the original.
    let roundtrips = |enc: &[u8]| {
        let back = message_from_wire(enc).unwrap().1;
        assert_eq!(
            message_to_wire(from, &back).unwrap(),
            default,
            "negotiated send must round-trip exactly"
        );
    };

    // FULL surface (both speak zstd): the bake-off crushes the bulk list below the default, and it
    // still decodes with no hint.
    let full = Negotiated {
        use_type_id: false,
        may_send_computed: true,
        compression: WireCompression::Zstd,
        may_use_cm: true,
        peer_max_bytes: 1 << 20,
    };
    let enc =
        message_to_wire_negotiated(from, &bulk, &full, WireTypeRegistry::new(Vec::new())).unwrap();
    assert!(
        enc.len() <= default.len(),
        "negotiated is never larger than the default"
    );
    assert!(
        enc.len() < default.len(),
        "the negotiated bake-off shrinks a 256-int list"
    );
    roundtrips(&enc);

    // A receiver that shares NO compression: the send never uses a codec it can't decode, still
    // round-trips, and is still ≤ default.
    let nocomp = Negotiated {
        compression: WireCompression::None,
        ..full
    };
    let enc_nc =
        message_to_wire_negotiated(from, &bulk, &nocomp, WireTypeRegistry::new(Vec::new()))
            .unwrap();
    assert!(enc_nc.len() <= default.len());
    roundtrips(&enc_nc);

    // TYPE-ID name elision fires ONLY when negotiated, shrinking a struct with long field names.
    use crate::interpreter::StructValue;
    let mut fields = HashMap::new();
    fields.insert("organization_identifier".to_string(), RuntimeValue::Int(1));
    fields.insert(
        "organization_display_name".to_string(),
        RuntimeValue::Text(Rc::new("ACME".to_string())),
    );
    let s = RuntimeValue::Struct(Box::new(StructValue {
        type_name: "Organization".to_string(),
        fields,
    }));
    let names = vec![
        "organization_display_name".to_string(),
        "organization_identifier".to_string(),
    ];
    let inline = message_to_wire(from, &s).unwrap();
    let with_id = Negotiated {
        use_type_id: true,
        ..nocomp
    };
    let elided = message_to_wire_negotiated(
        from,
        &s,
        &with_id,
        WireTypeRegistry::new(vec![("Organization".to_string(), names.clone())]),
    )
    .unwrap();
    assert!(
        elided.len() < inline.len(),
        "type-id elides the long field NAMES from the wire"
    );
    // It decodes when the receiver shares the same registry (the negotiated condition), to the same value.
    let back = with_type_registry(
        WireTypeRegistry::new(vec![("Organization".to_string(), names)]),
        || message_from_wire(&elided),
    )
    .unwrap()
    .1;
    assert_eq!(
        message_to_wire(from, &back).unwrap(),
        inline,
        "the struct round-trips through the shared registry"
    );

    // A computation the receiver DECLINED is refused at send (defense in depth).
    use crate::ast::stmt::{BinaryOpKind, Expr, Literal};
    use logicaffeine_base::{Arena, Symbol};
    let a: Arena<Expr> = Arena::new();
    let i = Symbol::from_index(0);
    let idx: &Expr = a.alloc(Expr::Identifier(i));
    let one: &Expr = a.alloc(Expr::Literal(Literal::Number(1)));
    let fbody: &Expr = a.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left: idx,
        right: one,
    });
    let gen = lower_expr_to_genexpr(fbody, i).unwrap();
    let f = RuntimeValue::Function(Box::new(ClosureValue {
        body_index: usize::MAX,
        captured_env: std::collections::HashMap::default(),
        param_names: vec![i],
        generated: Some(Rc::new(gen)),
    }));
    let declined = Negotiated {
        may_send_computed: false,
        ..full
    };
    assert!(
        message_to_wire_negotiated(from, &f, &declined, WireTypeRegistry::new(Vec::new())).is_err(),
        "a computation the receiver declined is refused at SEND"
    );
    let accepted = Negotiated {
        may_send_computed: true,
        ..full
    };
    assert!(
        message_to_wire_negotiated(from, &f, &accepted, WireTypeRegistry::new(Vec::new())).is_ok(),
        "an accepted computation is sent"
    );
}

#[test]
fn wire_type_registry_epoch_is_deterministic_and_distinguishes_type_sets() {
    // The registry epoch is the handshake's "do we share types?" key. Same types declared in any
    // field order → SAME epoch (so two same-program peers match and may elide names); a different
    // type set → a different epoch; empty → 0 (never elide).
    let a = WireTypeRegistry::new(vec![(
        "Org".to_string(),
        vec!["id".to_string(), "name".to_string()],
    )]);
    let b = WireTypeRegistry::new(vec![(
        "Org".to_string(),
        vec!["name".to_string(), "id".to_string()],
    )]);
    assert_eq!(
        a.epoch(),
        b.epoch(),
        "same types in any field order → identical epoch"
    );
    assert_ne!(a.epoch(), 0, "a non-empty registry is never epoch 0");
    let c = WireTypeRegistry::new(vec![("User".to_string(), vec!["id".to_string()])]);
    assert_ne!(
        a.epoch(),
        c.epoch(),
        "a different type set → a different epoch"
    );
    assert_eq!(
        WireTypeRegistry::new(vec![]).epoch(),
        0,
        "empty registry → epoch 0 (never elide)"
    );
    // Enums participate too: adding an enum changes the epoch, order-independently.
    let with_enum = WireTypeRegistry::new(vec![(
        "Org".to_string(),
        vec!["id".to_string(), "name".to_string()],
    )])
    .with_enums(vec![(
        "Color".to_string(),
        vec!["Red".to_string(), "Green".to_string()],
    )]);
    assert_ne!(
        a.epoch(),
        with_enum.epoch(),
        "adding an enum type changes the epoch"
    );
}

#[test]
fn handshake_frame_round_trips_and_is_never_confused_with_a_data_message() {
    let prof = PeerProfile {
        limits: ReceiveLimits {
            max_bytes: 4096,
            accept_computed: false,
            ..Default::default()
        },
        registry_epoch: 42,
        features: FEAT_LZ4 | FEAT_TYPE_ID,
    };
    let frame = make_handshake_frame("alice", &prof);
    assert_eq!(
        parse_handshake_frame(&frame),
        Some(("alice".to_string(), prof)),
        "handshake round-trips"
    );

    // A real data message is NEVER parsed as a handshake (the magic can't collide with a frame header).
    let data = message_to_wire("alice", &RuntimeValue::Int(7)).unwrap();
    assert!(
        parse_handshake_frame(&data).is_none(),
        "a data message is not a handshake"
    );
    // …and a handshake frame is not a decodable data message.
    assert!(
        message_from_wire(&frame).is_none(),
        "a handshake frame is not a data message"
    );
    // Truncations never panic or mis-parse.
    for cut in 0..frame.len() {
        let _ = parse_handshake_frame(&frame[..cut]);
    }
}

#[test]
fn wire_struct_list_type_id_elides_names_and_beats_inline() {
    // The BULK case: a homogeneous struct LIST. With a shared registry, the columnar
    // list ships its type id + N + columns — the type/field NAMES never go on the wire,
    // on the FIRST message. Strictly smaller than the self-describing `T_STRUCTS`.
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(
        (0..50).map(|i| point(i, i * 2)).collect(),
    ))));
    let schemas = vec![("Point".to_string(), vec!["x".to_string(), "y".to_string()])];
    let with_reg = with_type_registry(WireTypeRegistry::new(schemas.clone()), || {
        message_to_wire("p", &v).unwrap()
    });
    let inline = message_to_wire("p", &v).unwrap();
    assert!(
        with_reg.len() < inline.len(),
        "struct-list type-id ({}) must elide names vs inline ({})",
        with_reg.len(),
        inline.len()
    );
    let back = with_type_registry(WireTypeRegistry::new(schemas), || {
        message_from_wire(&with_reg).unwrap().1
    });
    assert_eq!(
        message_to_wire("p", &back).unwrap(),
        inline,
        "struct-list type-id round-trips to the exact value"
    );
}

#[test]
fn wire_struct_list_type_id_falls_back_for_unknown_type() {
    // A registry without this type → byte-identical self-describing `T_STRUCTS`.
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(
        (0..10).map(|i| point(i, i)).collect(),
    ))));
    let other = vec![("Other".to_string(), vec!["a".to_string()])];
    let bytes = with_type_registry(WireTypeRegistry::new(other), || {
        message_to_wire("p", &v).unwrap()
    });
    assert_eq!(
        bytes,
        message_to_wire("p", &v).unwrap(),
        "unknown type falls back to byte-identical inline T_STRUCTS"
    );
}

#[test]
fn wire_struct_list_type_id_unknown_id_fails_cleanly() {
    // Encoded against one registry, decoded against an EMPTY one: the id can't resolve
    // → None (clean), never a mis-decode.
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(
        (0..10).map(|i| point(i, i)).collect(),
    ))));
    let schemas = vec![("Point".to_string(), vec!["x".to_string(), "y".to_string()])];
    let bytes = with_type_registry(WireTypeRegistry::new(schemas), || {
        message_to_wire("p", &v).unwrap()
    });
    let decoded = with_type_registry(WireTypeRegistry::new(vec![]), || message_from_wire(&bytes));
    assert!(
        decoded.is_none(),
        "an unresolvable struct-list type-id must fail cleanly, not mis-decode"
    );
}

#[test]
fn wire_struct_type_id_registry_order_independent() {
    // The id is content-addressed (canonical by fingerprint), so two registries that
    // declare the same types in different ORDER assign the same ids — sender and
    // receiver always agree regardless of declaration order.
    let v = point(7, 9);
    let a = vec![
        ("Point".to_string(), vec!["x".to_string(), "y".to_string()]),
        ("Other".to_string(), vec!["a".to_string()]),
    ];
    let mut b = a.clone();
    b.reverse();
    let enc_a = with_type_registry(WireTypeRegistry::new(a), || {
        message_to_wire("p", &v).unwrap()
    });
    let dec_b = with_type_registry(WireTypeRegistry::new(b), || {
        message_from_wire(&enc_a).unwrap().1
    });
    assert_eq!(
        message_to_wire("p", &dec_b).unwrap(),
        message_to_wire("p", &v).unwrap()
    );
}

#[test]
fn wire_enum_type_id_elides_type_and_constructor_names() {
    // An enum (Inductive) under a shared registry ships its enum-id + a constructor
    // INDEX instead of the type and constructor NAMES — both ends know the ordered
    // constructor list from their shared type def. Strictly smaller than inline.
    let v = enum_val("Color", "Green", vec![]);
    let enums = vec![(
        "Color".to_string(),
        vec!["Red".to_string(), "Green".to_string(), "Blue".to_string()],
    )];
    let reg = || WireTypeRegistry::new(vec![]).with_enums(enums.clone());
    let with_reg = with_type_registry(reg(), || message_to_wire("p", &v).unwrap());
    let inline = message_to_wire("p", &v).unwrap();
    assert!(
        with_reg.len() < inline.len(),
        "enum type-id ({}) elides names vs inline ({})",
        with_reg.len(),
        inline.len()
    );
    let back = with_type_registry(reg(), || message_from_wire(&with_reg).unwrap().1);
    assert_eq!(
        message_to_wire("p", &back).unwrap(),
        inline,
        "enum type-id round-trips to the exact value"
    );
}

#[test]
fn wire_enum_type_id_carries_args_and_falls_back_when_unknown() {
    // A non-nullary constructor's args ride along; an enum NOT in the registry stays
    // the byte-identical self-describing inline form.
    let some = enum_val("Option", "Some", vec![RuntimeValue::Int(7)]);
    let enums = vec![(
        "Option".to_string(),
        vec!["None".to_string(), "Some".to_string()],
    )];
    let with_reg = with_type_registry(
        WireTypeRegistry::new(vec![]).with_enums(enums.clone()),
        || message_to_wire("p", &some).unwrap(),
    );
    let back = with_type_registry(WireTypeRegistry::new(vec![]).with_enums(enums), || {
        message_from_wire(&with_reg).unwrap().1
    });
    assert_eq!(
        message_to_wire("p", &back).unwrap(),
        message_to_wire("p", &some).unwrap()
    );
    // An unrelated registry (no Option) → byte-identical inline.
    let other =
        WireTypeRegistry::new(vec![]).with_enums(vec![("X".to_string(), vec!["A".to_string()])]);
    let bytes = with_type_registry(other, || message_to_wire("p", &some).unwrap());
    assert_eq!(
        bytes,
        message_to_wire("p", &some).unwrap(),
        "unknown enum falls back to inline"
    );
}

// ---- Pillar A: offset-table struct view (beat Cap'n Proto random access) --------

#[test]
fn wire_struct_view_round_trips() {
    let mut fields = HashMap::new();
    fields.insert("a".to_string(), RuntimeValue::Int(1));
    fields.insert("b".to_string(), RuntimeValue::Int(2));
    fields.insert("c".to_string(), RuntimeValue::Int(99));
    let v = RuntimeValue::Struct(Box::new(StructValue {
        type_name: "Rec".to_string(),
        fields,
    }));
    let bytes = with_struct_view(true, || message_to_wire("p", &v).unwrap());
    let back = message_from_wire(&bytes).unwrap().1;
    assert_eq!(
        message_to_wire("p", &back).unwrap(),
        message_to_wire("p", &v).unwrap(),
        "offset-table view round-trips to the exact struct"
    );
}

#[test]
fn wire_struct_view_reads_one_field_without_parsing_the_rest() {
    // Cap'n Proto-class random access: a struct with a HUGE field and a small field.
    // Reading the small field via the offset table is O(1) — a THOUSAND such reads must
    // be cheaper than ONE full decode, proving it never parses the huge field.
    let big: Vec<i64> = (0..1_000_000).map(|i| i as i64).collect();
    let mut fields = HashMap::new();
    fields.insert(
        "big".to_string(),
        RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(big)))),
    );
    fields.insert("small".to_string(), RuntimeValue::Int(4242));
    let v = RuntimeValue::Struct(Box::new(StructValue {
        type_name: "Rec".to_string(),
        fields,
    }));
    // The DEFAULT (checksummed) message — the view is STILL O(1): `view_message` does
    // not re-hash the body (that would defeat random access), so `Raw` is NOT required
    // for zero-copy. The offset-table field jump never touches the 1M-element field.
    let bytes = with_struct_view(true, || message_to_wire("p", &v).unwrap());

    let view = view_message(&bytes).unwrap();
    assert_eq!(
        view.struct_field("small").and_then(|f| f.as_int()),
        Some(4242),
        "the offset table reads the small field directly"
    );

    let reads = {
        let t = std::time::Instant::now();
        for _ in 0..1000 {
            let view = view_message(&bytes).unwrap();
            std::hint::black_box(view.struct_field("small").and_then(|f| f.as_int()));
        }
        t.elapsed().as_nanos()
    };
    let full = {
        let t = std::time::Instant::now();
        std::hint::black_box(message_from_wire(&bytes).unwrap());
        t.elapsed().as_nanos()
    };
    assert!(
        reads < full,
        "1000 O(1) view reads ({reads}ns) must beat ONE full decode ({full}ns) — capnp-class random access"
    );
}

#[test]
fn wire_aligned_int_column_reads_zero_copy_as_slice() {
    // The in-place column read — the LAN / kernel-bypass axis Cap'n Proto owns. An
    // aligned i64 column reads back as `&[i64]` with ZERO copy: the slice BORROWS the
    // message bytes, no allocation and no per-element decode, the way an io_uring / RDMA
    // receiver reads pre-registered, alignment-guaranteed buffers in place.
    let data: Vec<i64> = (0..1000).map(|i| i * 7 - 3).collect();
    let value = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(data.clone()))));
    let bytes = with_struct_view(true, || message_to_wire("p", &value).unwrap());

    // Place the framed message in an 8-byte-aligned buffer, as a real zero-copy receiver
    // would — a `Vec<i64>` allocation is 8-aligned by `i64`'s alignment, and the encoder
    // padded the column so its final-buffer offset is ≡ 0 mod 8.
    let mut backing = vec![0i64; bytes.len() / 8 + 2];
    // SAFETY: copy the message into the aligned backing's bytes; thereafter read-only.
    unsafe {
        std::ptr::copy_nonoverlapping(
            bytes.as_ptr(),
            backing.as_mut_ptr().cast::<u8>(),
            bytes.len(),
        );
    }
    let abytes: &[u8] =
        unsafe { std::slice::from_raw_parts(backing.as_ptr().cast::<u8>(), bytes.len()) };

    let view = view_message(abytes).unwrap();
    let slice = view
        .as_i64_slice()
        .expect("an aligned column reads zero-copy as &[i64]");
    assert_eq!(
        slice,
        &data[..],
        "the zero-copy slice equals the column data"
    );

    // It BORROWS the message buffer (zero allocation): the slice lives inside `abytes`
    // and is 8-byte aligned (a sound `&[i64]` cast on every architecture, not just x86).
    let base = abytes.as_ptr() as usize;
    let lo = slice.as_ptr() as usize;
    assert!(
        lo >= base && lo < base + abytes.len(),
        "the slice borrows the message bytes (zero-copy)"
    );
    assert_eq!(lo % 8, 0, "the column blob is 8-byte aligned");

    // The same bytes still round-trip through a full owned decode (the T_INTS_ALIGNED
    // decode arm), re-encoding to the exact aligned form.
    let back = message_from_wire(abytes).unwrap().1;
    let re = with_struct_view(true, || message_to_wire("p", &back).unwrap());
    assert_eq!(
        re, bytes,
        "the aligned column also decodes + re-encodes to the exact bytes"
    );
}

#[test]
fn wire_aligned_int_column_falls_back_to_copy_when_unaligned() {
    // When the receiver's buffer is NOT 8-aligned at the column, `as_i64_slice` returns
    // None (no UB) and the caller copies via the full decode — which still round-trips.
    let data: Vec<i64> = (0..64).map(|i| i - 32).collect();
    let value = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(data.clone()))));
    let bytes = with_struct_view(true, || message_to_wire("p", &value).unwrap());

    // Force a deliberate 1-byte misalignment by prepending a byte to an aligned backing,
    // so the message body starts at an odd offset and the column can't be 8-aligned.
    let mut backing = vec![0i64; bytes.len() / 8 + 2];
    let raw = unsafe {
        std::slice::from_raw_parts_mut(backing.as_mut_ptr().cast::<u8>(), bytes.len() + 1)
    };
    raw[1..bytes.len() + 1].copy_from_slice(&bytes);
    let shifted: &[u8] = &raw[1..bytes.len() + 1];

    let view = view_message(shifted).unwrap();
    // Either the misalignment made the cast unsound (→ None, copy fallback) — the
    // important invariant is no panic and a correct owned decode regardless.
    if let Some(slice) = view.as_i64_slice() {
        assert_eq!(
            slice.as_ptr() as usize % 8,
            0,
            "if it returned a slice it MUST be aligned"
        );
    }
    let back = message_from_wire(shifted).unwrap().1;
    let re = with_struct_view(true, || message_to_wire("p", &back).unwrap());
    assert_eq!(
        re, bytes,
        "the unaligned column still decodes correctly via copy"
    );
}

#[test]
fn wire_aligned_float_column_reads_zero_copy_as_slice() {
    // The float twin of the zero-copy i64 read: an aligned f64 column reads back as
    // `&[f64]` with no copy, BIT-EXACT — including NaN / ±Inf / subnormals, which the
    // `&[f64]` cast carries verbatim (no per-element decode could change a bit).
    let mut data: Vec<f64> = (0..1000).map(|i| i as f64 * 1.5 - 7.0).collect();
    data[3] = f64::NAN;
    data[5] = f64::INFINITY;
    data[7] = f64::NEG_INFINITY;
    data[9] = f64::MIN_POSITIVE / 4.0; // a subnormal
    data[11] = -0.0;
    let value = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(data.clone()))));
    let bytes = with_struct_view(true, || message_to_wire("p", &value).unwrap());

    let mut backing = vec![0i64; bytes.len() / 8 + 2];
    // SAFETY: copy the message into the 8-aligned backing's bytes; thereafter read-only.
    unsafe {
        std::ptr::copy_nonoverlapping(
            bytes.as_ptr(),
            backing.as_mut_ptr().cast::<u8>(),
            bytes.len(),
        );
    }
    let abytes: &[u8] =
        unsafe { std::slice::from_raw_parts(backing.as_ptr().cast::<u8>(), bytes.len()) };

    let view = view_message(abytes).unwrap();
    let slice = view
        .as_f64_slice()
        .expect("an aligned float column reads zero-copy as &[f64]");
    // Bit-exact comparison (NaN != NaN under `==`, so compare the raw bits).
    let got: Vec<u64> = slice.iter().map(|x| x.to_bits()).collect();
    let want: Vec<u64> = data.iter().map(|x| x.to_bits()).collect();
    assert_eq!(
        got, want,
        "the zero-copy float slice is bit-exact (NaN/Inf/subnormal/-0 preserved)"
    );

    // It BORROWS the message buffer (zero allocation) and is 8-byte aligned.
    let base = abytes.as_ptr() as usize;
    let lo = slice.as_ptr() as usize;
    assert!(
        lo >= base && lo < base + abytes.len(),
        "the float slice borrows the message bytes (zero-copy)"
    );
    assert_eq!(lo % 8, 0, "the float column blob is 8-byte aligned");

    // Full owned decode round-trips the exact aligned form too.
    let back = message_from_wire(abytes).unwrap().1;
    let re = with_struct_view(true, || message_to_wire("p", &back).unwrap());
    assert_eq!(
        re, bytes,
        "the aligned float column also decodes + re-encodes to the exact bytes"
    );
}

#[test]
fn wire_structs_view_round_trips() {
    // A record LIST in the random-access view round-trips to the exact same bytes.
    let mut rows = Vec::new();
    for i in 0..50i64 {
        let mut fields = HashMap::new();
        fields.insert("id".to_string(), RuntimeValue::Int(i));
        fields.insert("score".to_string(), RuntimeValue::Int(i * 3 - 7));
        fields.insert("active".to_string(), RuntimeValue::Bool(i % 2 == 0));
        rows.push(RuntimeValue::Struct(Box::new(StructValue {
            type_name: "Row".to_string(),
            fields,
        })));
    }
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(rows))));
    let bytes = with_struct_view(true, || message_to_wire("p", &v).unwrap());
    let back = message_from_wire(&bytes).unwrap().1;
    assert_eq!(
        with_struct_view(true, || message_to_wire("p", &back).unwrap()),
        bytes,
        "record-list view round-trips to the exact bytes"
    );
}

#[test]
fn wire_structs_view_reads_one_field_of_one_row_without_parsing_the_rest() {
    // O(1) random access into a record list: each row carries a big `blob` column, but
    // reading row r's small `id` must jump straight there via the row + field offset
    // tables — never materializing the blobs. The Cap'n Proto-class record-list read.
    let big: Vec<i64> = (0..1000).collect();
    let mut rows = Vec::new();
    for i in 0..200i64 {
        let mut fields = HashMap::new();
        fields.insert("id".to_string(), RuntimeValue::Int(i * 1000 + 1));
        fields.insert(
            "blob".to_string(),
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(big.clone())))),
        );
        rows.push(RuntimeValue::Struct(Box::new(StructValue {
            type_name: "Row".to_string(),
            fields,
        })));
    }
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(rows))));
    let bytes = with_struct_view(true, || message_to_wire("p", &v).unwrap());
    let view = view_message(&bytes).unwrap();

    assert_eq!(
        view.structs_len(),
        Some(200),
        "row count read from the view head"
    );
    assert_eq!(
        view.structs_row_field(7, "id").and_then(|f| f.as_int()),
        Some(7 * 1000 + 1),
        "O(1) read of row 7's id"
    );
    assert_eq!(
        view.structs_row_field(199, "id").and_then(|f| f.as_int()),
        Some(199 * 1000 + 1),
        "O(1) read of the last row's id"
    );
    assert!(
        view.structs_row_field(200, "id").is_none(),
        "row out of range → None"
    );
    assert!(
        view.structs_row_field(0, "nope").is_none(),
        "unknown field → None"
    );

    // The O(1) random reads beat ONE full decode (which materializes every blob column).
    let idxs: Vec<usize> = (0..1000).map(|k| (k * 7) % 200).collect();
    let reads = {
        let t = std::time::Instant::now();
        let mut acc = 0i64;
        for &i in &idxs {
            acc = acc.wrapping_add(view.structs_row_field(i, "id").unwrap().as_int().unwrap());
        }
        std::hint::black_box(acc);
        t.elapsed().as_nanos()
    };
    let full = {
        let t = std::time::Instant::now();
        std::hint::black_box(message_from_wire(&bytes).unwrap());
        t.elapsed().as_nanos()
    };
    assert!(
        reads < full,
        "1000 O(1) row-field reads ({reads}ns) must beat ONE full decode ({full}ns) — record-list random access"
    );
}

#[test]
fn wire_cyclic_value_fails_cleanly_instead_of_overflowing() {
    // A self-referential list (constructible via the `Rc<RefCell<…>>` a List wraps) must
    // NOT stack-overflow the recursive encoder — it returns a clean Err. Completeness /
    // robustness: the codec never crashes on a value, however pathological.
    let cell = Rc::new(RefCell::new(ListRepr::Boxed(vec![])));
    let list = RuntimeValue::List(cell.clone());
    *cell.borrow_mut() = ListRepr::Boxed(vec![list.clone()]); // the list now contains itself
    let result = message_to_wire("p", &list);
    *cell.borrow_mut() = ListRepr::Boxed(vec![]); // break the cycle so the Rc doesn't leak
    assert!(
        result.is_err(),
        "a cyclic value must return Err, not overflow the stack"
    );
}

#[test]
fn wire_deeply_nested_value_round_trips_below_the_guard() {
    // A legitimately deep (but finite) nesting still round-trips — the cycle guard only
    // rejects the pathological, never real data. Build via `from_values` so the value is
    // already canonical (the codec de-boxes on decode, so a hand-built `Boxed` would not
    // compare equal — that is canonicalization, not a round-trip failure).
    let mut v = RuntimeValue::Int(7);
    for _ in 0..40 {
        v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(vec![v]))));
    }
    let bytes = message_to_wire("p", &v).expect("deep-but-finite nesting encodes");
    let back = message_from_wire(&bytes).expect("deep nesting decodes").1;
    // Byte-stable round-trip (List equality is Rc-identity, so re-encode and compare the
    // canonical bytes — the idiom every round-trip lock-in in this file uses).
    assert_eq!(
        message_to_wire("p", &back).unwrap(),
        bytes,
        "deep nesting round-trips exactly"
    );
}

#[test]
fn wire_schema_def_decodes_without_a_cache() {
    // The FIRST cached message carries the schema inline (a "def"), so even a
    // stateless decoder handles it.
    let v = struct_list(20);
    let mut send_cache = WireSchemaCache::default();
    let m1 = message_to_wire_cached(
        "p",
        &v,
        WireCodec::Native,
        WireIntegrity::Raw,
        &mut send_cache,
    )
    .unwrap();
    let d = message_from_wire(&m1).unwrap().1;
    assert_eq!(
        message_to_wire("p", &d).unwrap(),
        message_to_wire("p", &v).unwrap()
    );
}

#[test]
fn wire_schema_ref_without_cache_fails_cleanly() {
    // A later message is a schema "ref"; a stateless decoder has no schema for that
    // id, so it returns None — never a panic, never a wrong value.
    let v = struct_list(20);
    let mut send_cache = WireSchemaCache::default();
    let _m1 = message_to_wire_cached(
        "p",
        &v,
        WireCodec::Native,
        WireIntegrity::Raw,
        &mut send_cache,
    )
    .unwrap();
    let m2 = message_to_wire_cached(
        "p",
        &v,
        WireCodec::Native,
        WireIntegrity::Raw,
        &mut send_cache,
    )
    .unwrap();
    assert!(
        message_from_wire(&m2).is_none(),
        "a schema-ref without a cache decodes to None"
    );
}

fn person(i: i64) -> RuntimeValue {
    let mut f = HashMap::new();
    f.insert(
        "name".to_string(),
        RuntimeValue::Text(Rc::new(format!("n{i}"))),
    );
    f.insert("score".to_string(), RuntimeValue::Int(i));
    RuntimeValue::Struct(Box::new(StructValue {
        type_name: "Person".to_string(),
        fields: f,
    }))
}

#[test]
fn wire_schema_dictionary_distinct_schemas_get_distinct_ids() {
    // Two different struct shapes each get their own schema id; interleaved sends
    // round-trip exactly through synchronized caches.
    let points = struct_list(10);
    let people = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
        (0..10).map(person).collect(),
    ))));
    let mut sc = WireSchemaCache::default();
    let mut rc = WireSchemaCache::default();
    let enc = |x: &RuntimeValue, c: &mut WireSchemaCache| {
        message_to_wire_cached("p", x, WireCodec::Native, WireIntegrity::Raw, c).unwrap()
    };
    let seq = [
        enc(&points, &mut sc),
        enc(&people, &mut sc),
        enc(&points, &mut sc),
        enc(&people, &mut sc),
    ];
    let originals = [&points, &people, &points, &people];
    for (bytes, orig) in seq.iter().zip(originals) {
        let d = message_from_wire_cached(bytes, &mut rc).unwrap().1;
        assert_eq!(
            message_to_wire("p", &d).unwrap(),
            message_to_wire("p", orig).unwrap()
        );
    }
}

#[test]
fn wire_schema_cache_handles_nested_struct_columns() {
    // A struct whose field is itself a list of structs — the NESTED schema is also
    // dictionaried (its own id), so the 2nd message is smaller and both round-trip.
    let inner = || {
        RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
            (0..3).map(|i| point(i, i)).collect(),
        ))))
    };
    let outer: Vec<RuntimeValue> = (0..5)
        .map(|i| {
            let mut f = HashMap::new();
            f.insert("id".to_string(), RuntimeValue::Int(i));
            f.insert("pts".to_string(), inner());
            RuntimeValue::Struct(Box::new(StructValue {
                type_name: "Bag".to_string(),
                fields: f,
            }))
        })
        .collect();
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(outer))));
    let mut sc = WireSchemaCache::default();
    let mut rc = WireSchemaCache::default();
    let enc = |c: &mut WireSchemaCache| {
        message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, c).unwrap()
    };
    let m1 = enc(&mut sc);
    let m2 = enc(&mut sc);
    assert!(
        m2.len() < m1.len(),
        "nested schemas reference on the 2nd message: {} vs {}",
        m2.len(),
        m1.len()
    );
    let d1 = message_from_wire_cached(&m1, &mut rc).unwrap().1;
    let d2 = message_from_wire_cached(&m2, &mut rc).unwrap().1;
    assert_eq!(
        message_to_wire("p", &d1).unwrap(),
        message_to_wire("p", &v).unwrap()
    );
    assert_eq!(
        message_to_wire("p", &d2).unwrap(),
        message_to_wire("p", &v).unwrap()
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn wire_schema_cache_composes_with_compression_and_checksum() {
    // Schema-by-reference + zstd + FNV checksum all stack: the ref message is
    // smaller and round-trips through the verified, inflated, schema-resolved path.
    let v = struct_list(300);
    let mut sc = WireSchemaCache::default();
    let mut rc = WireSchemaCache::default();
    let enc = |c: &mut WireSchemaCache| {
        with_compression_codec(WireCompression::Zstd, || {
            message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Checked, c).unwrap()
        })
    };
    let m1 = enc(&mut sc);
    let m2 = enc(&mut sc);
    assert!(
        m2.len() < m1.len(),
        "compressed+checked ref < def: {} vs {}",
        m2.len(),
        m1.len()
    );
    assert!(message_from_wire_cached(&m1, &mut rc).is_some());
    let d2 = message_from_wire_cached(&m2, &mut rc).unwrap().1;
    assert_eq!(
        message_to_wire("p", &d2).unwrap(),
        message_to_wire("p", &v).unwrap()
    );
}

#[test]
fn wire_schema_cache_fuzz_never_diverges_from_stateless() {
    // The proof: over many random message sequences through synchronized caches,
    // every decoded value equals the original (canonical stateless re-encode). The
    // cached protocol must never change the MEANING — only the bytes. Schemas
    // repeat, so definitions, references, and non-struct messages all interleave.
    fn gen_msg(rng: &mut SplitMix64) -> RuntimeValue {
        match rng.below(6) {
            0 => RuntimeValue::Int(rng.next() as i64),
            1 => RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(
                (0..rng.below(20) as i64).collect(),
            )))),
            2 => struct_list(rng.below(20) as i64),
            3 => RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
                (0..rng.below(20) as i64).map(person).collect(),
            )))),
            4 => RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
                (0..rng.below(15))
                    .map(|i| RuntimeValue::Text(Rc::new(format!("s{i}"))))
                    .collect(),
            )))),
            _ => RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
                (0..rng.below(12) as i64)
                    .map(|i| {
                        if i % 2 == 0 {
                            enum_val("Option", "Some", vec![RuntimeValue::Int(i)])
                        } else {
                            enum_val("Option", "None", vec![])
                        }
                    })
                    .collect(),
            )))),
        }
    }
    for seed in [1u64, 7, 42, 99, 1000, 0xDEAD_BEEF, 0x00AB_CDEF] {
        let mut rng = SplitMix64 { state: seed };
        let mut sc = WireSchemaCache::default();
        let mut rc = WireSchemaCache::default();
        for step in 0..120 {
            let v = gen_msg(&mut rng);
            let bytes =
                message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut sc)
                    .unwrap();
            let (_from, back) = message_from_wire_cached(&bytes, &mut rc)
                .unwrap_or_else(|| panic!("seed {seed} step {step}: cached decode returned None"));
            assert_eq!(
                message_to_wire("p", &back).unwrap(),
                message_to_wire("p", &v).unwrap(),
                "seed {seed} step {step}: cached round-trip changed the value"
            );
        }
    }
}

#[test]
fn wire_schema_content_addressed_survives_multi_sender_reorder_and_loss() {
    // THE FOOTGUN PROOF. Many senders share ONE receiver cache; the stream is
    // adversarially reordered and ~20% dropped. Every decode is EITHER exactly
    // correct OR None — never a wrong value. (Sequential ids would corrupt here;
    // content-addressing cannot, because the id IS the schema's content.)
    fn gen(rng: &mut SplitMix64) -> RuntimeValue {
        match rng.below(4) {
            0 => struct_list(rng.below(15) as i64),
            1 => RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
                (0..rng.below(15) as i64).map(person).collect(),
            )))),
            2 => RuntimeValue::Int(rng.next() as i64),
            _ => RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(
                (0..rng.below(15) as i64).collect(),
            )))),
        }
    }
    for seed in [3u64, 17, 71, 2024, 0xFEED, 0xBADF00D] {
        let mut rng = SplitMix64 { state: seed };
        // Three senders, each its OWN content-addressed send cache.
        let mut sends: Vec<WireSchemaCache> = (0..3)
            .map(|_| WireSchemaCache::content_addressed())
            .collect();
        let mut stream: Vec<(Vec<u8>, RuntimeValue)> = Vec::new();
        for _ in 0..80 {
            let s = rng.below(3) as usize;
            let v = gen(&mut rng);
            let bytes = message_to_wire_cached(
                "p",
                &v,
                WireCodec::Native,
                WireIntegrity::Raw,
                &mut sends[s],
            )
            .unwrap();
            stream.push((bytes, v));
        }
        // Adversary: Fisher–Yates shuffle (reordering) ...
        for i in (1..stream.len()).rev() {
            let j = rng.below((i + 1) as u64) as usize;
            stream.swap(i, j);
        }
        // ... and decode through ONE shared receiver, dropping ~20%.
        let mut recv = WireSchemaCache::content_addressed();
        for (bytes, orig) in &stream {
            if rng.below(5) == 0 {
                continue; // dropped before reaching the decoder (loss)
            }
            if let Some((_from, back)) = message_from_wire_cached(bytes, &mut recv) {
                assert_eq!(
                    message_to_wire("p", &back).unwrap(),
                    message_to_wire("p", orig).unwrap(),
                    "seed {seed}: a shared cache under reorder+loss decoded the WRONG value"
                );
            }
            // None is an acceptable outcome (a reference whose definition was
            // dropped or has not yet arrived) — it is a clean miss, not corruption.
        }
    }
}

#[test]
fn wire_schema_keyframe_self_heals_after_missed_definition() {
    // With a keyframe interval, the sender re-defines the schema every k references,
    // so a receiver that joined late (missed the first definition) recovers.
    let v = struct_list(10);
    let mut send = WireSchemaCache::content_addressed().with_keyframe(2);
    let msgs: Vec<Vec<u8>> = (0..6)
        .map(|_| {
            message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut send)
                .unwrap()
        })
        .collect();
    // Emission pattern is [Def, Ref, Ref, Def(keyframe), Ref, Ref].
    let mut recv = WireSchemaCache::content_addressed();
    assert!(
        message_from_wire_cached(&msgs[1], &mut recv).is_none(),
        "a reference before any definition → None"
    );
    assert!(
        message_from_wire_cached(&msgs[3], &mut recv).is_some(),
        "the keyframe re-definition decodes and heals the cache"
    );
    let d = message_from_wire_cached(&msgs[4], &mut recv).unwrap().1;
    assert_eq!(
        message_to_wire("p", &d).unwrap(),
        message_to_wire("p", &v).unwrap(),
        "references resolve after the keyframe"
    );
}

#[test]
fn wire_schema_mode_dial_trades_size() {
    // The dial: sequential (1-byte id) < content-addressed (8-byte fingerprint) <
    // inline (full schema). All three round-trip; smaller modes need more
    // discipline (sequential = one ordered sender), proven safe elsewhere.
    let v = struct_list(50);
    let ref_size = |mode_cache: fn() -> WireSchemaCache| {
        let mut c = mode_cache();
        let _def =
            message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut c).unwrap();
        message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut c)
            .unwrap()
            .len()
    };
    let seq = ref_size(WireSchemaCache::sequential);
    let ca = ref_size(WireSchemaCache::content_addressed);
    let inline = message_to_wire("p", &v).unwrap().len();
    assert!(
        seq < ca,
        "sequential ref ({seq}) < content-addressed ref ({ca})"
    );
    assert!(
        ca < inline,
        "content-addressed ref ({ca}) < inline schema ({inline})"
    );
    // Sequential round-trips on a single ordered stream.
    let mut s = WireSchemaCache::sequential();
    let mut r = WireSchemaCache::sequential();
    let m1 =
        message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut s).unwrap();
    let m2 =
        message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut s).unwrap();
    let d1 = message_from_wire_cached(&m1, &mut r).unwrap().1;
    let d2 = message_from_wire_cached(&m2, &mut r).unwrap().1;
    assert_eq!(
        message_to_wire("p", &d1).unwrap(),
        message_to_wire("p", &v).unwrap()
    );
    assert_eq!(
        message_to_wire("p", &d2).unwrap(),
        message_to_wire("p", &v).unwrap()
    );
}

#[test]
fn schema_cache_survives_a_panic_in_the_codec() {
    // The CacheScope RAII guard restores the thread-local cache on a panic unwind,
    // so a panic mid-codec can't strand or reset the schema state.
    let v = struct_list(20);
    let mut cache = WireSchemaCache::content_addressed();
    let _def =
        message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut cache).unwrap();
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _scope = CacheScope::enter(&mut cache);
        panic!("boom mid-codec");
    }));
    assert!(r.is_err(), "the panic propagated");
    // The cache still remembers the schema → the next send is a (smaller) reference.
    let after =
        message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut cache).unwrap();
    let fresh = {
        let mut f = WireSchemaCache::content_addressed();
        message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut f).unwrap()
    };
    assert!(
        after.len() < fresh.len(),
        "cache survived the panic (ref {} < fresh def {})",
        after.len(),
        fresh.len()
    );
}

#[test]
fn schema_cache_sequential_and_content_addressed_recv_are_disjoint() {
    // One receiver decoding interleaved sequential AND content-addressed messages
    // never cross-resolves — sequential uses recv_seq (ids), content uses recv_ca
    // (fingerprints), which are separate state. (Disproves the audit's "mode-mixing
    // collision" worry.)
    let v = struct_list(20);
    let mut seq = WireSchemaCache::sequential();
    let s_def =
        message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut seq).unwrap();
    let s_ref =
        message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut seq).unwrap();
    let mut ca = WireSchemaCache::content_addressed();
    let c_def =
        message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut ca).unwrap();
    let c_ref =
        message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut ca).unwrap();
    let mut rc = WireSchemaCache::content_addressed();
    for (bytes, label) in [
        (&s_def, "seq def"),
        (&c_def, "ca def"),
        (&s_ref, "seq ref"),
        (&c_ref, "ca ref"),
    ] {
        let d = message_from_wire_cached(bytes, &mut rc)
            .unwrap_or_else(|| panic!("{label} failed to decode"))
            .1;
        assert_eq!(
            message_to_wire("p", &d).unwrap(),
            message_to_wire("p", &v).unwrap(),
            "{label} reconstructs the list"
        );
    }
}

#[test]
fn wire_integrity_dial_toggles_the_checksum() {
    // The latency↔safety dial: `Raw` drops the 8-byte FNV checksum (faster, header
    // bit 0x01 unset), `Checked` keeps it. Scoped — never leaks.
    let v = struct_list(50);
    let raw = with_integrity(WireIntegrity::Raw, || message_to_wire("p", &v).unwrap());
    let checked = with_integrity(WireIntegrity::Checked, || message_to_wire("p", &v).unwrap());
    assert_eq!(raw[0] & 0x01, 0, "Raw carries no checksum");
    assert_eq!(checked[0] & 0x01, 0x01, "Checked sets the checksum bit");
    assert_eq!(checked.len(), raw.len() + 8, "the checksum is 8 bytes");
    assert!(
        message_from_wire(&raw).is_some() && message_from_wire(&checked).is_some(),
        "both decode"
    );
    // Scoped: outside the override the process default is restored.
    let default_bit = if default_integrity() == WireIntegrity::Checked {
        0x01
    } else {
        0
    };
    assert_eq!(
        message_to_wire("p", &v).unwrap()[0] & 0x01,
        default_bit,
        "the override does not leak"
    );
}

#[test]
fn uvarint_byte_len_matches_write_uvarint() {
    for x in [
        0u64,
        1,
        127,
        128,
        16_383,
        16_384,
        u32::MAX as u64,
        u64::MAX,
        0x1234_5678_9ABC_DEF0,
    ] {
        let mut buf = Vec::new();
        write_uvarint(x, &mut buf);
        assert_eq!(
            uvarint_byte_len(x),
            buf.len(),
            "uvarint_byte_len({x:#x}) must match write_uvarint"
        );
    }
}

#[test]
fn wire_json_codec_is_real_json() {
    let v = RuntimeValue::Text(Rc::new("hi".to_string()));
    let bytes = message_to_wire_with("alice", &v, WireCodec::Json, WireIntegrity::Raw).unwrap();
    // The body after the 1-byte header parses as JSON via a real parser.
    let json: serde_json::Value = serde_json::from_slice(&bytes[1..]).expect("valid JSON body");
    assert_eq!(json["from"], "alice");
}

#[test]
fn wire_native_is_far_tighter_than_json() {
    // The throughput win: a 100-int array is a fraction of the JSON size.
    let items: Vec<RuntimeValue> = (0..100).map(RuntimeValue::Int).collect();
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(items))));
    let bin = message_to_wire_with("", &v, WireCodec::Native, WireIntegrity::Raw).unwrap();
    let json = message_to_wire_with("", &v, WireCodec::Json, WireIntegrity::Raw).unwrap();
    assert!(
        bin.len() * 2 < json.len(),
        "native ({} bytes) should be far tighter than json ({} bytes)",
        bin.len(),
        json.len()
    );
}

#[test]
fn wire_decoder_never_panics_on_arbitrary_bytes() {
    let mut rng = SplitMix64 { state: 0x1234_5678 };
    for _ in 0..5000 {
        let len = rng.below(80) as usize;
        let bytes: Vec<u8> = (0..len).map(|_| (rng.next() & 0xFF) as u8).collect();
        let _ = message_from_wire(&bytes); // must not panic
    }
    // Every possible header byte with a short body.
    for h in 0u16..=255 {
        let _ = message_from_wire(&[h as u8, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
    }
}

#[test]
fn wire_decoder_never_panics_on_mutated_valid_messages() {
    // Random bytes almost never form a valid tag, so they never reach the deep
    // columnar / schema-cache paths. Here we take a VALID message of each archetype
    // and mutate it every which way — truncate at every length, flip every byte to
    // a few values, scramble bytes — and assert the decoder NEVER panics (it
    // returns None or a structurally-valid Some that itself re-encodes without
    // panicking). This is the robustness floor the whole protocol rests on.
    fn archetypes() -> Vec<RuntimeValue> {
        vec![
            RuntimeValue::Int(42),
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints((0..5).collect())))),
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(
                (0..5).map(|i| i as f64 * 1.5).collect(),
            )))),
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Bools(
                (0..5).map(|i| i % 2 == 0).collect(),
            )))),
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
                (0..4)
                    .map(|i| RuntimeValue::Text(Rc::new(format!("s{i}"))))
                    .collect(),
            )))),
            struct_list(4),
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
                (0..6)
                    .map(|i| {
                        if i % 2 == 0 {
                            enum_val("Option", "Some", vec![RuntimeValue::Int(i)])
                        } else {
                            enum_val("Option", "None", vec![])
                        }
                    })
                    .collect(),
            )))),
        ]
    }
    // A mutated buffer must never panic; if it decodes, the value re-encodes.
    let check = |bytes: &[u8]| {
        if let Some((_from, v)) = message_from_wire(bytes) {
            let _ = message_to_wire("p", &v);
        }
        let mut c = WireSchemaCache::content_addressed();
        if let Some((_from, v)) = message_from_wire_cached(bytes, &mut c) {
            let _ = message_to_wire("p", &v);
        }
    };
    let mut rng = SplitMix64 { state: 0x00AB_CDEF };
    for v in archetypes() {
        // Plain, cached-def, and cached-ref encodings all get mutated.
        let mut sc = WireSchemaCache::content_addressed();
        let bases = [
            message_to_wire("p", &v).unwrap(),
            message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut sc)
                .unwrap(),
            message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut sc)
                .unwrap(),
        ];
        for base in &bases {
            check(base); // the valid form decodes
            for k in 0..base.len() {
                check(&base[..k]); // every truncation
            }
            for i in 0..base.len() {
                for delta in [0x01u8, 0x40, 0x7F, 0x80, 0xFF] {
                    let mut m = base.clone();
                    m[i] ^= delta;
                    check(&m); // every single-byte flip
                }
            }
            for _ in 0..30 {
                let mut m = base.clone();
                if !m.is_empty() {
                    let i = rng.below(m.len() as u64) as usize;
                    m[i] = (rng.next() & 0xFF) as u8;
                }
                check(&m); // random scramble
            }
        }
    }
}

#[test]
fn wire_property_random_values_are_byte_stable() {
    for seed in [1u64, 2, 7, 42, 99, 1000, 0x00AB_CDEF, 0xDEAD_BEEF] {
        let mut rng = SplitMix64 { state: seed };
        for _ in 0..150 {
            let v = gen_value(&mut rng, 4);
            assert_wire_stable(&v);
        }
    }
}

#[test]
fn wire_packed_int_array_is_tight_and_stays_packed() {
    let n = 5000i64;
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints((0..n).collect()))));
    assert_wire_stable(&v);
    let bytes = message_to_wire_with("", &v, WireCodec::Native, WireIntegrity::Raw).unwrap();
    // Packed: ~2 bytes/int at these magnitudes — far under a tagged element each.
    assert!(
        bytes.len() < n as usize * 3,
        "packed ints should be tight, was {} bytes",
        bytes.len()
    );
    match message_from_wire(&bytes).unwrap().1 {
        RuntimeValue::List(l) => assert!(
            matches!(&*l.borrow(), ListRepr::Ints(_)),
            "decodes to a packed Ints buffer"
        ),
        other => panic!("expected a list, got {other:?}"),
    }
}

#[test]
fn wire_fixed_width_int_mode_is_memcpy_and_interops() {
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints((0..1000).collect()))));
    let varint = message_to_wire("", &v).unwrap();
    let fixed = with_fixed_numerics(|| message_to_wire("", &v).unwrap());
    // Fixed-width is the raw i64 bytes — bigger than varint (no compression),
    // but a memcpy to load (8 bytes/int + small framing).
    assert!(
        fixed.len() > varint.len(),
        "fixed-width should be larger than varint"
    );
    assert!(
        fixed.len() >= 1000 * 8,
        "fixed is 8 bytes/int, was {}",
        fixed.len()
    );
    // Both encodings interoperate — the decoder handles either tag.
    let vals = |bytes: &[u8]| match message_from_wire(bytes).unwrap().1 {
        RuntimeValue::List(l) => l.borrow().to_values(),
        other => panic!("expected a list, got {other:?}"),
    };
    let expected: Vec<RuntimeValue> = (0..1000).map(RuntimeValue::Int).collect();
    assert_eq!(vals(&varint), expected);
    assert_eq!(vals(&fixed), expected);
    // Scoped: the mode does not leak past `with_fixed_numerics`.
    assert_eq!(message_to_wire("", &v).unwrap(), varint);
}

#[test]
fn gv_simd_decode_matches_scalar_oracle_over_fuzz() {
    // The SSSE3 fast path must be bit-identical to the scalar `gv_decode`
    // oracle. We sweep magnitudes so every 2-bit width code {1,2,4,8 bytes}
    // and thus every adjacent `(code_a, code_b)` shuffle-mask is exercised,
    // and lengths so the even SIMD body, the odd tail, and the near-buffer-end
    // tail all fire. `gv_decode_dispatch` takes the SIMD path on this host.
    for seed in [1u64, 2, 7, 42, 99, 1000, 0xDEAD_BEEF, 0x00AB_CDEF] {
        let mut rng = SplitMix64 { state: seed };
        for _ in 0..200 {
            let n = (rng.below(37)) as usize; // 0, odd, and >16-int blocks
            let vals: Vec<i64> = (0..n)
                .map(|_| {
                    let bits = rng.below(64) as u32; // span all four width buckets
                    let mask = (1u128 << bits).wrapping_sub(1) as u64;
                    let mag = (rng.next() & mask) as i64;
                    if rng.next() & 1 == 0 {
                        mag
                    } else {
                        -mag
                    }
                })
                .collect();
            let mut buf = Vec::new();
            gv_encode(&mut buf, vals.iter().copied(), vals.len());

            let (mut p1, mut p2) = (0usize, 0usize);
            let scalar = gv_decode(&buf, &mut p1).expect("scalar decode");
            let simd = gv_decode_dispatch(&buf, &mut p2).expect("dispatch decode");

            assert_eq!(scalar, vals, "scalar oracle lost data (seed {seed}, n {n})");
            assert_eq!(simd, vals, "simd/dispatch lost data (seed {seed}, n {n})");
            assert_eq!(
                p1, p2,
                "decoders consumed a different byte count (seed {seed})"
            );
        }
    }
}

#[test]
fn wire_group_varint_mode_interops_with_varint() {
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(
        (0..1000).map(|i| i * i - 500_000).collect(),
    ))));
    let varint = message_to_wire("", &v).unwrap();
    let gv = with_numerics(WireNumerics::GroupVarint, || {
        message_to_wire("", &v).unwrap()
    });
    // The numeric tag is self-describing — either encoding decodes to the same
    // values, regardless of which strategy produced it.
    let vals = |bytes: &[u8]| match message_from_wire(bytes).unwrap().1 {
        RuntimeValue::List(l) => l.borrow().to_values(),
        other => panic!("expected a list, got {other:?}"),
    };
    assert_eq!(vals(&varint), vals(&gv));
    let expected: Vec<RuntimeValue> = (0..1000)
        .map(|i| RuntimeValue::Int(i * i - 500_000))
        .collect();
    assert_eq!(vals(&gv), expected);
    // Scoped: the mode does not leak past `with_numerics`.
    assert_eq!(message_to_wire("", &v).unwrap(), varint);
}

#[test]
fn wire_packed_float_array_roundtrips_bit_exact() {
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(vec![
        0.0,
        -0.0,
        1.5,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::MIN,
        f64::MAX,
    ]))));
    assert_wire_stable(&v); // byte-stability ⇒ bit-exact, NaN included
    match message_from_wire(&message_to_wire("", &v).unwrap())
        .unwrap()
        .1
    {
        RuntimeValue::List(l) => assert!(matches!(&*l.borrow(), ListRepr::Floats(_))),
        other => panic!("expected a list, got {other:?}"),
    }
}

fn floats_list(vals: Vec<f64>) -> RuntimeValue {
    RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(vals))))
}
fn float_bits(v: &RuntimeValue) -> Vec<u64> {
    match v {
        RuntimeValue::List(l) => match &*l.borrow() {
            ListRepr::Floats(f) => f.iter().map(|x| x.to_bits()).collect(),
            other => panic!("not a float list: {other:?}"),
        },
        other => panic!("not a list: {other:?}"),
    }
}

#[test]
fn wire_floats_xor_is_bit_exact_including_special_values() {
    // The XOR-delta float codec operates on raw bits, so it is LOSSLESS and
    // bit-exact for every f64 — NaN payloads, ±Inf, ±0.0, subnormals included.
    let vals = vec![
        1.0,
        1.0000001,
        1.0000002,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        -0.0,
        0.0,
        f64::MIN,
        f64::MAX,
        f64::MIN_POSITIVE,
    ];
    let v = floats_list(vals.clone());
    let bytes = with_floats(WireFloats::XorDelta, || message_to_wire("p", &v).unwrap());
    let back = message_from_wire(&bytes).unwrap().1;
    let orig: Vec<u64> = vals.iter().map(|x| x.to_bits()).collect();
    assert_eq!(
        float_bits(&back),
        orig,
        "XOR-delta float column is bit-exact"
    );
}

#[test]
fn wire_floats_xor_shrinks_slow_varying() {
    // Consecutive samples share their high bits, so XOR-delta + varint is far
    // smaller than 8 bytes/elem memcpy — the time-series win.
    let vals: Vec<f64> = (0..1000).map(|i| 100.0 + i as f64 * 1e-6).collect();
    let v = floats_list(vals);
    let memcpy = message_to_wire("p", &v).unwrap();
    let xor = with_floats(WireFloats::XorDelta, || message_to_wire("p", &v).unwrap());
    assert!(
        xor.len() < memcpy.len(),
        "XOR-delta shrinks a slow-varying column: {} vs {}",
        xor.len(),
        memcpy.len()
    );
    assert_eq!(
        float_bits(&message_from_wire(&xor).unwrap().1),
        float_bits(&message_from_wire(&memcpy).unwrap().1)
    );
}

#[test]
fn wire_floats_xor_never_grows() {
    // A high-entropy column: XOR-delta would be larger, so the encoder falls back
    // to memcpy — never bigger than the baseline.
    let mut rng = SplitMix64 { state: 12345 };
    let vals: Vec<f64> = (0..1000).map(|_| f64::from_bits(rng.next())).collect();
    let v = floats_list(vals);
    let memcpy = message_to_wire("p", &v).unwrap();
    let xor = with_floats(WireFloats::XorDelta, || message_to_wire("p", &v).unwrap());
    assert!(
        xor.len() <= memcpy.len(),
        "XOR-delta must never grow vs memcpy: {} vs {}",
        xor.len(),
        memcpy.len()
    );
    assert_eq!(
        float_bits(&message_from_wire(&xor).unwrap().1),
        float_bits(&message_from_wire(&memcpy).unwrap().1)
    );
}

#[test]
fn wire_floats_xor_fuzz_bit_identical() {
    // Random columns mixing slow-varying runs and arbitrary bit patterns: the XOR-delta decode is the
    // original's wire bits, bit-identical, every NaN the canonical NaN. Arbitrary bit patterns draw NaNs with
    // every sign and payload (one draw in 2048), and the seeds together must draw some.
    use logicaffeine_data::wire::{canonical_f64_bits, CANONICAL_NAN_BITS};
    let mut payload_nans = 0usize;
    for seed in [1u64, 7, 42, 1000, 0xBEEF, 0xC0FFEE] {
        let mut rng = SplitMix64 { state: seed };
        for _ in 0..200 {
            let n = rng.below(30) as usize;
            let base = f64::from_bits(rng.next());
            let vals: Vec<f64> = (0..n)
                .map(|i| {
                    if rng.below(2) == 0 {
                        base + i as f64 * 1e-9
                    } else {
                        f64::from_bits(rng.next())
                    }
                })
                .collect();
            let v = floats_list(vals.clone());
            let xor = with_floats(WireFloats::XorDelta, || message_to_wire("p", &v).unwrap());
            let orig: Vec<u64> = vals.iter().map(|x| canonical_f64_bits(*x)).collect();
            let got = float_bits(&message_from_wire(&xor).unwrap().1);
            assert_eq!(got, orig, "seed {seed}: XOR-delta diverged");
            for (x, g) in vals.iter().zip(&got) {
                if x.is_nan() {
                    payload_nans += usize::from(x.to_bits() != CANONICAL_NAN_BITS);
                    assert_eq!(*g, CANONICAL_NAN_BITS, "seed {seed}: the NaN {:#x} decodes as the canonical NaN", x.to_bits());
                } else {
                    assert_eq!(*g, x.to_bits(), "seed {seed}: {x:?} keeps its exact bits");
                }
            }
        }
    }
    assert!(payload_nans > 0, "the columns drew no NaN with a non-canonical payload");
    let edge = vec![-0.0, 0.0, f64::from_bits(0xFFF8_0000_0000_0001), -0.0, 1.5, f64::from_bits(0x7FF0_0000_0000_0001)];
    let xor = with_floats(WireFloats::XorDelta, || message_to_wire("p", &floats_list(edge)).unwrap());
    let want = vec![(-0.0f64).to_bits(), 0, CANONICAL_NAN_BITS, (-0.0f64).to_bits(), 1.5f64.to_bits(), CANONICAL_NAN_BITS];
    assert_eq!(float_bits(&message_from_wire(&xor).unwrap().1), want, "-0.0 keeps its sign bit and every NaN is canonical");
}

#[test]
fn wire_packed_bool_array_is_bit_packed() {
    let n = 1000usize;
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Bools(
        (0..n).map(|i| i % 3 == 0).collect(),
    ))));
    assert_wire_stable(&v);
    let bytes = message_to_wire_with("", &v, WireCodec::Native, WireIntegrity::Raw).unwrap();
    // 8 booleans per byte → ~125 bytes for 1000, not ~1000.
    assert!(
        bytes.len() < n / 4,
        "bools must be bit-packed: {} bytes for {} bools",
        bytes.len(),
        n
    );
    match message_from_wire(&bytes).unwrap().1 {
        RuntimeValue::List(l) => assert!(matches!(&*l.borrow(), ListRepr::Bools(_))),
        other => panic!("expected a list, got {other:?}"),
    }
}

#[test]
fn wire_mixed_list_stays_generic() {
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(vec![
        RuntimeValue::Int(1),
        RuntimeValue::Text(Rc::new("x".to_string())),
        RuntimeValue::Bool(true),
    ]))));
    assert_wire_stable(&v);
    match message_from_wire(&message_to_wire("", &v).unwrap())
        .unwrap()
        .1
    {
        RuntimeValue::List(l) => assert!(
            matches!(&*l.borrow(), ListRepr::Boxed(_)),
            "a mixed list stays Boxed"
        ),
        other => panic!("expected a list, got {other:?}"),
    }
}

#[test]
fn wire_string_array_packs_flat_and_loads_flat() {
    let strs = ["alpha", "", "héllo", "日本語", "emoji😀", "z"];
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
        strs.iter()
            .map(|s| RuntimeValue::Text(Rc::new(s.to_string())))
            .collect(),
    ))));
    // Byte-stable: Boxed-of-Text → T_STRINGS → Strings → T_STRINGS (same bytes).
    assert_wire_stable(&v);
    let bytes = message_to_wire_with("", &v, WireCodec::Native, WireIntegrity::Raw).unwrap();
    match message_from_wire(&bytes).unwrap().1 {
        RuntimeValue::List(l) => {
            let b = l.borrow();
            assert!(
                matches!(&*b, ListRepr::Strings { .. }),
                "a string array loads FLAT, not per-element"
            );
            assert_eq!(b.len(), strs.len());
            let got: Vec<String> = b
                .to_values()
                .into_iter()
                .map(|x| match x {
                    RuntimeValue::Text(s) => (*s).clone(),
                    other => panic!("expected text, got {other:?}"),
                })
                .collect();
            assert_eq!(got, strs);
            // Indexed access (get) also materializes the right element.
            assert!(matches!(b.get(2), Some(RuntimeValue::Text(s)) if s.as_str() == "héllo"));
        }
        other => panic!("expected a list, got {other:?}"),
    }
}

#[test]
fn wire_flat_strings_memoize_repeated_get() {
    // Best-of-both: load flat, but a *repeated* get of the same element returns
    // the cached `Rc` (a refcount bump) rather than re-materializing the String.
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(vec![
        RuntimeValue::Text(Rc::new("a".to_string())),
        RuntimeValue::Text(Rc::new("bb".to_string())),
        RuntimeValue::Text(Rc::new("ccc".to_string())),
    ]))));
    let back = message_from_wire(&message_to_wire("", &v).unwrap())
        .unwrap()
        .1;
    let RuntimeValue::List(l) = back else {
        panic!("expected a list")
    };
    let b = l.borrow();
    let (RuntimeValue::Text(first), RuntimeValue::Text(again)) =
        (b.get(1).unwrap(), b.get(1).unwrap())
    else {
        panic!("expected text")
    };
    assert_eq!(first.as_str(), "bb");
    assert!(
        Rc::ptr_eq(&first, &again),
        "a repeated get must reuse the memoized Rc, not re-allocate"
    );
}

#[test]
fn wire_mixed_with_one_nonstring_stays_generic() {
    // All-string ⇒ flat; one non-string ⇒ falls back to the tagged list.
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(vec![
        RuntimeValue::Text(Rc::new("a".to_string())),
        RuntimeValue::Int(1),
        RuntimeValue::Text(Rc::new("c".to_string())),
    ]))));
    assert_wire_stable(&v);
    match message_from_wire(&message_to_wire("", &v).unwrap())
        .unwrap()
        .1
    {
        RuntimeValue::List(l) => assert!(
            matches!(&*l.borrow(), ListRepr::Boxed(_)),
            "mixed stays Boxed"
        ),
        other => panic!("expected a list, got {other:?}"),
    }
}

#[test]
fn wire_i32_packed_list_roundtrips_through_ints() {
    // A half-width IntsI32 buffer encodes through the same packed path; it
    // rebuilds as full-width Ints (same values, same bytes — byte-stable).
    let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::IntsI32(vec![
        1, -2, 3, -4, 100,
    ]))));
    assert_wire_stable(&v);
    let (_, back) = message_from_wire(&message_to_wire("", &v).unwrap()).unwrap();
    match back {
        RuntimeValue::List(l) => match &*l.borrow() {
            ListRepr::Ints(got) => assert_eq!(got, &vec![1i64, -2, 3, -4, 100]),
            other => panic!("expected Ints, got {other:?}"),
        },
        other => panic!("expected a list, got {other:?}"),
    }
}

/// A running (non-ignored) report: for every payload × codec it prints the wire
/// size + ratios and a light throughput sample, AND asserts the deterministic
/// invariants (round-trip, native<json, zstd≤deflate, lz4 shrinks, columnar
/// structs are compact). Run with `--nocapture` to read the numbers; the asserts
/// hold either way. zstd is native-only, so this is a native-target test.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn wire_codec_report() {
    use std::time::Instant;

    fn enc(v: &RuntimeValue, comp: WireCompression, num: WireNumerics) -> Vec<u8> {
        with_numerics(num, || {
            with_compression_codec(comp, || message_to_wire("p", v).unwrap())
        })
    }

    let ints = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints((0..1000).collect()))));
    let floats = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(
        (0..1000).map(|i| i as f64 * 1.5).collect(),
    ))));
    let bools = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Bools(
        (0..1000).map(|i| i % 3 == 0).collect(),
    ))));
    let strings = redundant_list(200);
    let records = struct_list(1000);
    let payloads: [(&str, &RuntimeValue); 5] = [
        ("1000 ints", &ints),
        ("1000 floats", &floats),
        ("1000 bools", &bools),
        ("200 redundant strings", &strings),
        ("1000 structs (Point)", &records),
    ];

    println!("\n=== WIRE CODEC REPORT ===");
    for (name, v) in payloads {
        let raw = enc(v, WireCompression::None, WireNumerics::Varint);
        let json = message_to_wire_with("p", v, WireCodec::Json, WireIntegrity::Raw).unwrap();
        println!(
            "\n{name}: native {} B | json {} B  ({:.1}× tighter than json)",
            raw.len(),
            json.len(),
            json.len() as f64 / raw.len() as f64
        );
        for comp in [
            WireCompression::None,
            WireCompression::Deflate,
            WireCompression::Lz4,
            WireCompression::Zstd,
        ] {
            let b = enc(v, comp, WireNumerics::Varint);
            assert!(
                message_from_wire(&b).is_some(),
                "{name}/{comp:?} must round-trip"
            );
            let cn = format!("{comp:?}");
            println!(
                "  {cn:<8} {:>7} B  ({:.2}× of native raw)",
                b.len(),
                b.len() as f64 / raw.len() as f64
            );
        }
        assert!(raw.len() < json.len(), "{name}: native must beat json");
    }

    // Compression invariants on a hard-to-compress-no-more redundant payload.
    let red = redundant_list(1000);
    let raw = enc(&red, WireCompression::None, WireNumerics::Varint);
    let deflate = enc(&red, WireCompression::Deflate, WireNumerics::Varint);
    let lz4 = enc(&red, WireCompression::Lz4, WireNumerics::Varint);
    let zstd = enc(&red, WireCompression::Zstd, WireNumerics::Varint);
    assert!(
        lz4.len() < raw.len(),
        "lz4 shrinks redundant data ({} vs {})",
        lz4.len(),
        raw.len()
    );
    assert!(deflate.len() < raw.len(), "deflate shrinks redundant data");
    assert!(
        zstd.len() <= deflate.len(),
        "zstd ratio ≤ deflate ({} vs {})",
        zstd.len(),
        deflate.len()
    );

    // Columnar structs are far smaller than the old per-row-boxed form.
    assert!(
        records_is_compact(&records),
        "columnar struct list must be compact"
    );

    // Light throughput sample (encode+decode), so the numbers are visible.
    for (name, v) in [("1000 ints", &ints), ("1000 structs", &records)] {
        let it = 2000u32;
        let t = Instant::now();
        let mut total = 0usize;
        for _ in 0..it {
            let b = message_to_wire("p", v).unwrap();
            total += message_from_wire(&b).map(|_| b.len()).unwrap();
        }
        let el = t.elapsed();
        println!(
            "  throughput {name:<13}: {:>9.0} msg/s  {:>7.1} MB/s",
            it as f64 / el.as_secs_f64(),
            total as f64 / el.as_secs_f64() / 1e6
        );
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn records_is_compact(records: &RuntimeValue) -> bool {
    let b = message_to_wire("p", records).unwrap();
    b.len() < 6000
}

#[test]
#[ignore = "throughput benchmark — run with: cargo test -p logicaffeine-compile marshal::tests::bench_wire_throughput --release -- --ignored --nocapture"]
fn bench_wire_throughput() {
    use bincode::Options;
    use std::time::Instant;

    fn bench<F: FnMut() -> usize>(label: &str, iters: u32, mut f: F) {
        for _ in 0..(iters / 10).max(1) {
            f();
        } // warm up
        let t = Instant::now();
        let mut bytes = 0usize;
        for _ in 0..iters {
            bytes += f();
        }
        let el = t.elapsed();
        println!(
            "  {label:<26} {:>11.0} msg/s  {:>9.1} MB/s  ({:>8.0?}/op)",
            iters as f64 / el.as_secs_f64(),
            bytes as f64 / el.as_secs_f64() / 1e6,
            el / iters
        );
    }

    // bincode-of-WireValue — an off-the-shelf binary format, for comparison.
    let bincode_enc = |v: &RuntimeValue| {
        let p = materialize(v).unwrap();
        let msg = rt_to_wire(&p).unwrap();
        wire_options()
            .serialize(&WireMessage {
                from: "p".to_string(),
                msg,
            })
            .unwrap()
    };

    // Payloads across the type space.
    let mk_record = |i: i64| {
        let mut f = HashMap::new();
        f.insert("id".to_string(), RuntimeValue::Int(i));
        f.insert(
            "name".to_string(),
            RuntimeValue::Text(Rc::new(format!("item-{i}"))),
        );
        f.insert("active".to_string(), RuntimeValue::Bool(i % 2 == 0));
        RuntimeValue::Struct(Box::new(StructValue {
            type_name: "Record".to_string(),
            fields: f,
        }))
    };
    let ints = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints((0..1000).collect()))));
    let floats = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(
        (0..1000).map(|i| i as f64 * 1.5).collect(),
    ))));
    let bools = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Bools(
        (0..1000).map(|i| i % 3 == 0).collect(),
    ))));
    let strings = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
        (0..200)
            .map(|i| RuntimeValue::Text(Rc::new(format!("string-value-{i}"))))
            .collect(),
    ))));
    let records = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
        (0..200).map(mk_record).collect(),
    ))));

    let payloads: [(&str, &RuntimeValue); 5] = [
        ("1000 ints", &ints),
        ("1000 floats", &floats),
        ("1000 bools", &bools),
        ("200 strings", &strings),
        ("200 records", &records),
    ];

    for (name, v) in payloads {
        let nat = message_to_wire_with("p", v, WireCodec::Native, WireIntegrity::Raw).unwrap();
        let json = message_to_wire_with("p", v, WireCodec::Json, WireIntegrity::Raw).unwrap();
        let binc = bincode_enc(v);
        println!(
            "\n=== {name}: native {} B | bincode {} B | json {} B  ({:.1}× tighter than json) ===",
            nat.len(),
            binc.len(),
            json.len(),
            json.len() as f64 / nat.len() as f64
        );
        let it = 100_000;
        bench("native encode (raw)", it, || {
            message_to_wire_with("p", v, WireCodec::Native, WireIntegrity::Raw)
                .unwrap()
                .len()
        });
        bench("native encode (checked)", it, || {
            message_to_wire_with("p", v, WireCodec::Native, WireIntegrity::Checked)
                .unwrap()
                .len()
        });
        bench("bincode encode", it, || bincode_enc(v).len());
        bench("json encode", it, || {
            message_to_wire_with("p", v, WireCodec::Json, WireIntegrity::Raw)
                .unwrap()
                .len()
        });
        // Fixed-width numeric mode (memcpy) — only differs for int arrays.
        let nat_fixed = with_fixed_numerics(|| {
            message_to_wire_with("p", v, WireCodec::Native, WireIntegrity::Raw).unwrap()
        });
        if nat_fixed != nat {
            println!(
                "  (fixed-width numerics: {} B vs varint {} B)",
                nat_fixed.len(),
                nat.len()
            );
            bench("native encode (fixed)", it, || {
                with_fixed_numerics(|| {
                    message_to_wire_with("p", v, WireCodec::Native, WireIntegrity::Raw)
                        .unwrap()
                        .len()
                })
            });
            bench("native decode (fixed)", it, || {
                message_from_wire(&nat_fixed).unwrap();
                nat_fixed.len()
            });
        }
        // Group-varint numeric mode (SSSE3 shuffle decode) — also int-only. The
        // decode line is the headline: does SIMD beat LEB128 at varint size?
        let nat_gv = with_numerics(WireNumerics::GroupVarint, || {
            message_to_wire_with("p", v, WireCodec::Native, WireIntegrity::Raw).unwrap()
        });
        if nat_gv != nat {
            println!(
                "  (group-varint numerics: {} B vs varint {} B)",
                nat_gv.len(),
                nat.len()
            );
            bench("native encode (gv)", it, || {
                with_numerics(WireNumerics::GroupVarint, || {
                    message_to_wire_with("p", v, WireCodec::Native, WireIntegrity::Raw)
                        .unwrap()
                        .len()
                })
            });
            bench("native decode (gv/simd)", it, || {
                message_from_wire(&nat_gv).unwrap();
                nat_gv.len()
            });
        }
        bench("native decode (raw)", it, || {
            message_from_wire(&nat).unwrap();
            nat.len()
        });
        bench("bincode decode", it, || {
            let _: WireMessage = wire_options().deserialize(&binc).unwrap();
            binc.len()
        });
        bench("json decode", it, || {
            message_from_wire(&json).unwrap();
            json.len()
        });
    }
}

// --- A seeded generator of arbitrary (network-portable) values ----------

struct SplitMix64 {
    state: u64,
}
impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn gen_char(rng: &mut SplitMix64) -> char {
    loop {
        if let Some(c) = char::from_u32((rng.next() % 0x11_0000) as u32) {
            return c;
        }
    }
}

fn gen_string(rng: &mut SplitMix64) -> String {
    (0..rng.below(6)).map(|_| gen_char(rng)).collect()
}

fn gen_value(rng: &mut SplitMix64, depth: u32) -> RuntimeValue {
    // At depth 0 only scalars (indices 0..12); deeper, containers too.
    let kinds = if depth == 0 { 12 } else { 18 };
    match rng.below(kinds) {
        0 => RuntimeValue::Int(rng.next() as i64),
        1 => RuntimeValue::Float(f64::from_bits(rng.next())),
        2 => RuntimeValue::Bool(rng.next() & 1 == 0),
        3 => RuntimeValue::Char(gen_char(rng)),
        4 => RuntimeValue::Text(Rc::new(gen_string(rng))),
        5 => RuntimeValue::Nothing,
        6 => RuntimeValue::Duration(rng.next() as i64),
        7 => RuntimeValue::Date(rng.next() as i32),
        8 => RuntimeValue::Moment(rng.next() as i64),
        9 => RuntimeValue::Span {
            months: rng.next() as i32,
            days: rng.next() as i32,
        },
        10 => RuntimeValue::Time(rng.next() as i64),
        11 => RuntimeValue::Peer(Rc::new(gen_string(rng))),
        12 => RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(
            (0..rng.below(4))
                .map(|_| gen_value(rng, depth - 1))
                .collect(),
        )))),
        13 => RuntimeValue::Tuple(Rc::new(
            (0..rng.below(4))
                .map(|_| gen_value(rng, depth - 1))
                .collect(),
        )),
        14 => RuntimeValue::Set(Rc::new(RefCell::new(
            (0..rng.below(4))
                .map(|_| gen_value(rng, depth - 1))
                .collect(),
        ))),
        15 => {
            let mut m = MapStorage::default();
            for _ in 0..rng.below(4) {
                // Keys are scalars (depth 0) so they stay hashable + simple.
                m.insert(gen_value(rng, 0), gen_value(rng, depth - 1));
            }
            RuntimeValue::Map(Rc::new(RefCell::new(m)))
        }
        16 => {
            let mut fields = HashMap::new();
            for i in 0..rng.below(4) {
                fields.insert(format!("f{i}"), gen_value(rng, depth - 1));
            }
            RuntimeValue::Struct(Box::new(StructValue {
                type_name: format!("T{}", rng.below(5)),
                fields,
            }))
        }
        _ => RuntimeValue::Inductive(std::rc::Rc::new(InductiveValue {
            inductive_type: format!("I{}", rng.below(5)),
            constructor: format!("C{}", rng.below(5)),
            args: (0..rng.below(4))
                .map(|_| gen_value(rng, depth - 1))
                .collect(),
        })),
    }
}

/// A real number a function of the program defines stays with the program that defines it: a task's payload and the
/// wire each refuse it, saying why, where a computable real built from its exact forms crosses both.
#[test]
fn a_real_a_function_defines_is_never_sent() {
    struct Sevenths;
    impl logicaffeine_base::exact_real::Approximant for Sevenths {
        fn call(&self, n: u64) -> Result<logicaffeine_base::Rational, String> {
            let digit = if n == 0 { 0 } else { [1, 4, 2, 8, 5, 7][((n - 1) % 6) as usize] };
            Ok(logicaffeine_base::Rational::from_i64(digit))
        }
    }
    use logicaffeine_base::exact_real::{Definition, ExactReal};
    let definition = Definition::named("digits", 10).expect("a base from 2 up");
    let real = RuntimeValue::ExactReal(Rc::new(ExactReal::defined(
        definition,
        "seventh",
        std::sync::Arc::new(Sevenths),
    )));
    match materialize(&real) {
        Err(MarshalError::NotSendable(why)) => assert_eq!(why, "a real number a function of the program defines"),
        Err(other) => panic!("refused for another reason: {other:?}"),
        Ok(_) => panic!("a defined real became a task's payload"),
    }
    let mut out = Vec::new();
    let refused = native_encode(&real, &mut out).expect_err("a defined real went on the wire");
    assert!(refused.contains("stays with its program"), "{refused}");

    let pi = RuntimeValue::ExactReal(Rc::new(ExactReal::pi()));
    assert!(materialize(&pi).is_ok(), "a computable real of exact forms is a payload");
    assert!(native_encode(&pi, &mut Vec::new()).is_ok(), "a computable real of exact forms crosses the wire");
}

// ─────────────────────────────────────────────────────────────────────────────
// Number-theoretic and entropy-coded column forms on the wire (the `Auto` menu).
// ─────────────────────────────────────────────────────────────────────────────

/// SplitMix64 — deterministic, nonlinear test data.
struct Mix(u64);

impl Mix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> i64 {
        (self.next() % n) as i64
    }
}

fn auto_wire(v: &RuntimeValue) -> Vec<u8> {
    with_structure(WireStructure::Auto, || {
        message_to_wire_with("", v, WireCodec::Native, WireIntegrity::Raw).expect("encodes")
    })
}

/// The same encode at the `Max` effort level, where the column menu adds the entropy-coded form.
fn max_wire(v: &RuntimeValue) -> Vec<u8> {
    with_compression_level(WireCompressionLevel::Max, || auto_wire(v))
}

fn wire_floats(bytes: &[u8]) -> Vec<f64> {
    let (_, v) = message_from_wire(bytes).expect("decodes");
    let RuntimeValue::List(l) = v else { panic!("a list") };
    let values = l.borrow().to_values();
    values.into_iter().map(|x| match x {
        RuntimeValue::Float(f) => f,
        other => panic!("a float, got {other:?}"),
    }).collect()
}

fn wire_ints(bytes: &[u8]) -> Vec<i64> {
    let (_, v) = message_from_wire(bytes).expect("decodes");
    let RuntimeValue::List(l) = v else { panic!("a list") };
    let values = l.borrow().to_values();
    values.into_iter().map(|x| match x {
        RuntimeValue::Int(i) => i,
        other => panic!("an int, got {other:?}"),
    }).collect()
}

#[test]
fn a_decimal_float_column_ships_as_its_integer_column_under_auto() {
    let mut rng = Mix(21);
    let mut k = 215i64;
    let v: Vec<f64> = (0..20_000).map(|_| { k += rng.below(5) - 2; k as f64 / 10.0 }).collect();
    // The structural menu already ships the integer column; the Max effort level entropy-codes it.
    for (level, bytes) in [("default", auto_wire(&floats_list(v.clone()))), ("max", max_wire(&floats_list(v.clone())))] {
        assert_eq!(describe_columns(&bytes), vec!["decimal floats".to_string()], "{level}");
        let back = wire_floats(&bytes);
        assert!(back.iter().zip(&v).all(|(a, b)| a.to_bits() == b.to_bits()) && back.len() == v.len(), "{level}");
    }
    let structural = auto_wire(&floats_list(v.clone())).len();
    let full = max_wire(&floats_list(v.clone())).len();
    assert!(structural * 4 < v.len() * 8, "default: {structural} B for {} decimal doubles", v.len());
    assert!(full < v.len() / 2, "max: {full} B for {} decimal doubles — under half a byte each", v.len());
}

#[test]
fn full_precision_floats_take_the_bits_form_when_it_is_smaller() {
    // Draws from a pool of 300 full-precision doubles: no decimal exponent fits, no closed form fits,
    // but the bit patterns recur, so the integer menu over the patterns wins.
    let mut rng = Mix(22);
    let pool: Vec<f64> = (0..300).map(|_| f64::from_bits(0x4040_0000_0000_0000 | (rng.next() >> 14))).collect();
    let v: Vec<f64> = (0..20_000).map(|_| pool[rng.below(300) as usize]).collect();
    let bytes = auto_wire(&floats_list(v.clone()));
    assert_eq!(describe_columns(&bytes), vec!["float bits".to_string()]);
    let back = wire_floats(&bytes);
    assert!(back.iter().zip(&v).all(|(a, b)| a.to_bits() == b.to_bits()) && back.len() == v.len());
    assert!(bytes.len() * 4 < v.len() * 8, "{} B for {} recurring doubles", bytes.len(), v.len());
}

#[test]
fn the_decimal_and_bits_forms_write_the_canonical_nan() {
    let decimal = |nan: f64| {
        let mut v: Vec<f64> = (0..4_000).map(|i| (i % 731) as f64 / 100.0).collect();
        v[17] = nan;
        v[2_500] = nan;
        v
    };
    let mut rng = Mix(23);
    let pool: Vec<f64> = (0..200).map(|_| f64::from_bits(0x4040_0000_0000_0000 | (rng.next() >> 14))).collect();
    let picks: Vec<usize> = (0..4_000).map(|_| rng.below(200) as usize).collect();
    let bits = |nan: f64| {
        let mut v: Vec<f64> = picks.iter().map(|&i| pool[i]).collect();
        v[40] = nan;
        v
    };
    let nans = [-f64::NAN, f64::from_bits(0x7FF8_0000_0000_0001), f64::from_bits(0xFFF0_0000_0000_0001), f64::from_bits(u64::MAX)];
    for (name, make, form) in [("decimal", &decimal as &dyn Fn(f64) -> Vec<f64>, "decimal floats"), ("bits", &bits, "float bits")] {
        let want = auto_wire(&floats_list(make(f64::NAN)));
        assert_eq!(describe_columns(&want), vec![form.to_string()], "{name}: the column takes the {form} form");
        for nan in nans {
            assert_eq!(auto_wire(&floats_list(make(nan))), want, "{name}: NaN {:#x} encodes its own bits", nan.to_bits());
        }
        let back = wire_floats(&want);
        assert_eq!(back[if name == "decimal" { 17 } else { 40 }].to_bits(), f64::NAN.to_bits());
    }
}

#[test]
fn an_int_column_with_a_common_factor_ships_factored() {
    let mut rng = Mix(24);
    let mut t = 1_672_531_200i64;
    let v: Vec<i64> = (0..20_000).map(|_| { t += rng.below(90) - 30; t * 1_000_000 }).collect();
    let bytes = auto_wire(&ints_list(v.clone()));
    assert_eq!(describe_columns(&bytes), vec!["common-multiple".to_string()]);
    assert_eq!(wire_ints(&bytes), v);
}

#[test]
fn a_skewed_int_column_ships_entropy_coded() {
    let mut rng = Mix(25);
    let v: Vec<i64> = (0..20_000).map(|_| { let mut k = 0; while rng.next() % 1000 >= 50 { k += 1; } k }).collect();
    let bytes = max_wire(&ints_list(v.clone()));
    assert_eq!(describe_columns(&bytes), vec!["entropy-coded".to_string()]);
    assert_ne!(describe_columns(&auto_wire(&ints_list(v.clone()))), vec!["entropy-coded".to_string()], "only at Max");
    assert_eq!(wire_ints(&bytes), v);
    // h(0.05)/0.05 ≈ 5.73 bits a value; the column must land near it, not at a byte a value.
    assert!(bytes.len() * 8 < v.len() * 6, "{} B for {} geometric draws", bytes.len(), v.len());
}

#[test]
fn a_receiver_refuses_a_column_past_its_element_budget_whatever_its_form() {
    // A monotone column takes a delta form, a byte column the raw byte form: both decode under a budget
    // that admits them and are refused under one that does not.
    let monotone: Vec<i64> = (0..2_000).map(|i| i * 3 + (i % 7)).collect();
    let bytes_col: Vec<i64> = { let mut rng = Mix(26); (0..2_000).map(|_| rng.below(256)).collect() };
    for v in [monotone, bytes_col] {
        let wire = auto_wire(&ints_list(v.clone()));
        let tight = ReceiveLimits { max_elements: 1_999, ..DEFAULT_RECEIVE_LIMITS };
        let enough = ReceiveLimits { max_elements: 2_000, ..DEFAULT_RECEIVE_LIMITS };
        assert!(with_receive_limits(tight, || message_from_wire(&wire)).is_none(), "{:?}: refused past the budget", describe_columns(&wire));
        assert_eq!(with_receive_limits(enough, || wire_ints(&wire)), v);
    }
}

#[test]
fn column_tag_name_names_the_number_theoretic_and_entropy_coded_forms() {
    use logicaffeine_base::describe;
    for (tag, want) in [
        (describe::T_INTS_MULT, "common-multiple"),
        (describe::T_INTS_CM, "entropy-coded"),
        (describe::T_FLOATS_DECIMAL, "decimal floats"),
        (describe::T_FLOATS_BITS, "float bits"),
    ] {
        assert_eq!(column_tag_name(tag), want, "tag {tag}");
    }
    for &(name, tag) in describe::COLUMN_TAGS {
        assert_ne!(column_tag_name(tag), "value", "{name} ({tag}) has no plain-words name");
    }
}

#[test]
fn smallest_searches_the_max_effort_level_and_never_loses_to_a_single_dial() {
    // `Smallest` now tries every compressor at the Max level too and the entropy-coded column form, so it
    // is never larger than any single dial at either level — and on a skewed column it is the CM form.
    let mut rng = Mix(27);
    let v: Vec<i64> = (0..20_000).map(|_| { let mut k = 0; while rng.next() % 1000 >= 50 { k += 1; } k }).collect();
    let value = ints_list(v.clone());
    let best = message_to_wire_best("", &value, WireGoal::Smallest).unwrap();
    for level in [WireCompressionLevel::Balanced, WireCompressionLevel::Max] {
        for comp in [WireCompression::None, WireCompression::Deflate, WireCompression::Lz4, WireCompression::Zstd] {
            let single = with_compression_level(level, || {
                with_compression_codec(comp, || with_structure(WireStructure::Auto, || message_to_wire("", &value).unwrap()))
            });
            assert!(best.len() <= single.len(), "{level:?} {comp:?}: {} > {}", best.len(), single.len());
        }
    }
    assert_eq!(wire_ints(&best), v);
}

// ─────────────────────────────────────────────────────────────────────────────
// The context-mixing byte codec (`WireCompression::Cm`, header codec id 3).
// ─────────────────────────────────────────────────────────────────────────────

fn prose(n: usize) -> String {
    const WORDS: [&str; 20] = [
        "the", "receiver", "decodes", "every", "column", "exactly", "as", "it", "was", "written", "and", "a",
        "sender", "chooses", "the", "smallest", "form", "that", "round", "trips",
    ];
    let mut rng = Mix(41);
    let mut out = String::with_capacity(n + 16);
    while out.len() < n {
        out.push_str(WORDS[rng.below(WORDS.len() as u64) as usize]);
        out.push(if rng.below(11) == 0 { '\n' } else { ' ' });
    }
    out
}

#[test]
fn the_context_mixing_codec_round_trips_through_the_wire_header() {
    let text = RuntimeValue::Text(Rc::new(prose(60_000)));
    let wire = with_compression_codec(WireCompression::Cm, || message_to_wire_with("p", &text, WireCodec::Native, WireIntegrity::Checked).unwrap());
    let (_, compression, _) = unframe(&wire).expect("frames");
    assert_eq!(compression, WireCompression::Cm, "codec id 3 on the header");
    let (from, back) = message_from_wire(&wire).expect("decodes");
    assert_eq!(from, "p");
    assert!(matches!(back, RuntimeValue::Text(t) if *t == prose(60_000)));
    let zstd = with_compression_level(WireCompressionLevel::Max, || {
        with_compression_codec(WireCompression::Zstd, || message_to_wire_with("p", &text, WireCodec::Native, WireIntegrity::Checked).unwrap())
    });
    assert!(wire.len() < zstd.len(), "context mixing {} B vs zstd at its max level {} B", wire.len(), zstd.len());
}

#[test]
fn the_context_mixing_codec_refuses_a_body_past_its_decode_budget() {
    // A context-mixing decode costs about a microsecond a byte, so its output is capped well below the
    // general decompression budget: the sender never uses it past the cap, and a receiver refuses a frame
    // that claims more.
    let big = vec![b'a'; MAX_CM_DECODE_BYTES + 1];
    assert!(compress_body(WireCompression::Cm, &big).is_none(), "the sender keeps CM to bodies within the cap");
    let mut forged = Vec::new();
    write_uvarint((MAX_CM_DECODE_BYTES + 1) as u64, &mut forged);
    forged.extend_from_slice(&[0u8; 16]);
    assert!(decompress_body(WireCompression::Cm, &forged).is_none(), "a frame claiming more is refused before decoding");
    let small = b"within the budget".repeat(100);
    let (used, z) = compress_body(WireCompression::Cm, &small).unwrap();
    assert_eq!(used, WireCompression::Cm);
    assert_eq!(decompress_body(WireCompression::Cm, &z).as_deref(), Some(small.as_slice()));
}

#[test]
fn smallest_reaches_the_context_mixing_codec_at_the_max_level() {
    let text = RuntimeValue::Text(Rc::new(prose(40_000)));
    let best = message_to_wire_best("", &text, WireGoal::Smallest).unwrap();
    let (_, compression, _) = unframe(&best).unwrap();
    assert_eq!(compression, WireCompression::Cm, "on prose the context-mixing codec is the smallest");
    let (_, back) = message_from_wire(&best).unwrap();
    assert!(matches!(back, RuntimeValue::Text(t) if *t == prose(40_000)));
}

#[test]
fn negotiation_keeps_zstd_the_default_and_advertises_context_mixing_separately() {
    // CM is a spend-CPU-for-size codec, so a negotiated send never defaults to it; the feature bit only
    // tells a sender the receiver can decode it.
    let both = PeerProfile::default();
    let neg = negotiate(&both, &both);
    assert_eq!(neg.compression, WireCompression::Zstd);
    assert!(neg.may_use_cm);
    let without = PeerProfile { features: both.features & !FEAT_CM, ..both };
    assert!(!negotiate(&both, &without).may_use_cm);
    assert!(!negotiate(&both, &PeerProfile::conservative()).may_use_cm);
    assert_eq!(decode_peer_profile(&encode_peer_profile(&both)), Some(both), "the bit survives the handshake");
}

#[test]
fn the_column_name_table_names_each_tag_once_and_each_name_once() {
    use std::collections::HashSet;
    let tags: HashSet<u8> = COLUMN_TAG_NAMES.iter().map(|&(t, _)| t).collect();
    let names: HashSet<&str> = COLUMN_TAG_NAMES.iter().map(|&(_, n)| n).collect();
    assert_eq!(tags.len(), COLUMN_TAG_NAMES.len(), "a tag appears twice in COLUMN_TAG_NAMES");
    assert_eq!(names.len(), COLUMN_TAG_NAMES.len(), "two tags share a name in COLUMN_TAG_NAMES");
    assert!(!names.contains("value"), "the generic fallback is not an encoding's name");
    for &(tag, name) in COLUMN_TAG_NAMES {
        assert_eq!(column_tag_name(tag), name);
    }
}
