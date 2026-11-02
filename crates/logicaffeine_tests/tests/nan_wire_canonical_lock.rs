//! No LOGOS program can tell two NaNs apart.
//!
//! Every path a float takes to program output is blind to a NaN's sign and payload except the wire codec,
//! which writes a float's bytes: `wireBytes`, the sealed and framed forms, the relay and persistence
//! transports. So the encoders write every NaN as the one canonical quiet NaN, `0x7FF8_0000_0000_0000`,
//! in every shape a float can take (a scalar, a list element, a list column under every structure
//! dial, a record field, a complex part), and every other float keeps its exact bits, `-0.0` included.
//! A NaN with a payload that `fromWireBytes` built is re-encoded canonically too. The NaN a host's
//! arithmetic makes (x86 gives `0xFFF8…`, other hosts and WASM differ) is then unobservable, on every
//! tier and every host.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_data::wire::{WireDecode, WireEncode};

/// The canonical NaN's bytes, little-endian.
const CANON: [u8; 8] = [0, 0, 0, 0, 0, 0, 0xF8, 0x7F];

/// NaN sources, each a LOGOS expression: arithmetic on this host, a text parse, and a payload-carrying NaN
/// decoded from bytes.
const SOURCES: &[&str] = &[
    "sqrt(0.0 - 1.0)",
    "(0.0 - sqrt(0.0 - 1.0))",
    "((2.0 ** 1024.0) - (2.0 ** 1024.0))",
    "(0.0 * (2.0 ** 1024.0))",
    "parseFloat(\"nan\")",
    "parseFloat(\"-nan\")",
    "fromWireBytes([4, 1, 0, 0, 0, 0, 0, 248, 127])",
    "fromWireBytes([4, 255, 255, 255, 255, 255, 255, 255, 255])",
];

/// The shapes a float takes on the wire, each a function of the float expression `x`.
fn shapes(x: &str) -> Vec<(&'static str, String)> {
    vec![
        ("a scalar", x.to_string()),
        ("a list element", format!("[{x}, 1.5]")),
        ("a long column", format!("[{x}, 1.5, 2.5, 3.5, 4.5, 5.5, 6.5, 7.5, 8.5, 9.5, 10.5, 11.5, 12.5, {x}]")),
        ("a constant column", format!("[{x}, {x}, {x}, {x}, {x}, {x}, {x}, {x}]")),
        ("a record field", format!("a new Pt with x {x}")),
        ("a record list", format!("[a new Pt with x {x}, a new Pt with x 1.5, a new Pt with x {x}]")),
    ]
}

fn program(expr: &str) -> String {
    format!("## A Pt has:\n    An x: Real.\n\n## Main\n    Show wireBytes({expr}).\n")
}

/// Every tier the lock runs in process, the tree-walker and the VM: each one's printed lines (the tree-walker
/// joins lines without a final newline, the VM ends each with one) and its failure.
fn outcomes(src: &str) -> Vec<(&'static str, String)> {
    let vm = vm_outcome(src);
    let tw = tw_outcome(src);
    let shown = |output: &str, error: &Option<String>| format!("{:?} / {error:?}", output.trim_end_matches('\n'));
    vec![("vm", shown(&vm.output, &vm.error)), ("tree-walker", shown(&tw.output, &tw.error))]
}

#[test]
fn every_nan_source_has_the_same_wire_bytes_in_every_shape() {
    let mut bad = Vec::new();
    for (shape, _) in shapes("x") {
        let reference = shapes(SOURCES[0]).into_iter().find(|(s, _)| *s == shape).expect("the shape").1;
        let want = outcomes(&program(&reference));
        for source in SOURCES {
            let expr = shapes(source).into_iter().find(|(s, _)| *s == shape).expect("the shape").1;
            let got = outcomes(&program(&expr));
            for ((tier, w), (_, g)) in want.iter().zip(&got) {
                if g != w {
                    bad.push(format!("{shape} of {source} on the {tier}: {g}, where {} gives {w}", SOURCES[0]));
                }
            }
        }
    }
    assert!(bad.is_empty(), "{} NaN encodings differ:\n{}", bad.len(), bad.join("\n"));
}

#[test]
fn a_nan_scalar_is_the_canonical_quiet_nan_on_the_wire() {
    let want = format!("[4, {}]", CANON.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(", "));
    for source in SOURCES {
        for (tier, got) in outcomes(&program(source)) {
            assert_eq!(got, format!("{want:?} / None"), "{source} on the {tier}");
        }
    }
}

#[test]
fn every_other_float_keeps_its_exact_bits() {
    let values = [
        ("0.0", 0.0f64),
        ("(0.0 * (0.0 - 1.0))", -0.0),
        ("1.5", 1.5),
        ("(0.0 - 2.25)", -2.25),
        ("(2.0 ** 1024.0)", f64::INFINITY),
        ("(0.0 - (2.0 ** 1024.0))", f64::NEG_INFINITY),
        ("(2.0 ** (0.0 - 1074.0))", 5e-324),
    ];
    for (expr, value) in values {
        let bytes: Vec<String> = std::iter::once(4u8).chain(value.to_le_bytes()).map(|b| b.to_string()).collect();
        let want = format!("[{}]", bytes.join(", "));
        for (tier, got) in outcomes(&program(expr)) {
            assert_eq!(got, format!("{want:?} / None"), "{expr} keeps its bits on the {tier}");
        }
    }
}

#[test]
fn the_rust_runtime_codec_writes_the_canonical_nan_and_keeps_every_other_float() {
    let nans = [f64::NAN, -f64::NAN, f64::from_bits(0x7FF8_0000_0000_0001), f64::from_bits(0xFFF0_0000_0000_0001), f64::from_bits(u64::MAX)];
    for nan in nans {
        let mut out = Vec::new();
        nan.wire_encode(&mut out);
        assert_eq!(&out[1..], &CANON, "{:#x} encodes as the canonical NaN", nan.to_bits());
        let mut pos = 0;
        let back = f64::wire_decode(&out, &mut pos).expect("decodes");
        assert!(back.is_nan());
        let mut vector = Vec::new();
        vec![1.5, nan, -0.0].wire_encode(&mut vector);
        let mut expected = Vec::new();
        vec![1.5, f64::NAN, -0.0].wire_encode(&mut expected);
        assert_eq!(vector, expected, "{:#x} inside a vector encodes as the canonical NaN", nan.to_bits());
    }
    for keep in [0.0f64, -0.0, 1.5, f64::INFINITY, f64::NEG_INFINITY, 5e-324, f64::MAX] {
        let mut out = Vec::new();
        keep.wire_encode(&mut out);
        assert_eq!(&out[1..], &keep.to_le_bytes(), "{keep:?} keeps its bits");
    }
}

/// Columns that reach every float encoding: a raw column, a constant one, an affine one, a sparse one, a
/// periodic one and a slowly-varying one (the XOR-delta dial's case), each holding a NaN where `nan` goes.
fn columns(nan: f64) -> Vec<Vec<f64>> {
    let mut raw: Vec<f64> = (0..40).map(|i| (i as f64) * 1.37 - 5.0).collect();
    raw[7] = nan;
    let constant = vec![nan; 32];
    let mut affine: Vec<f64> = (0..32).map(|i| 1.0 + (i as f64) * 0.5).collect();
    affine[31] = nan;
    let mut sparse = vec![2.5; 64];
    sparse[3] = nan;
    sparse[40] = nan;
    let periodic: Vec<f64> = (0..48).map(|i| [1.0, nan, 3.0][i % 3]).collect();
    let mut slow: Vec<f64> = (0..64).map(|i| 100.0 + (i as f64) * 1e-9).collect();
    slow[20] = nan;
    vec![raw, constant, affine, sparse, periodic, slow]
}

#[test]
fn every_column_encoding_under_every_dial_writes_the_canonical_nan() {
    use logicaffeine_compile::concurrency::marshal::{
        build_columnar_record, encode_value_raw, message_to_wire_with, with_floats, with_numerics, with_structure, with_struct_view,
        WireCodec, WireColumn, WireFloats, WireIntegrity, WireNumerics, WireStructure,
    };
    use logicaffeine_compile::interpreter::{ListRepr, RuntimeValue};
    use std::cell::RefCell;
    use std::rc::Rc;
    let list = |v: Vec<f64>| RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(v))));
    let records = |v: Vec<f64>| {
        let ids = ListRepr::Ints((0..v.len() as i64).collect());
        let columns = vec![ids, ListRepr::Floats(v)];
        RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Structs { type_name: "Pt".into(), field_names: vec!["id".into(), "x".into()], columns })))
    };
    let nans = [-f64::NAN, f64::from_bits(0x7FF8_0000_0000_0001), f64::from_bits(0xFFF0_0000_0000_0001), f64::from_bits(u64::MAX)];
    let mut bad = Vec::new();
    for view in [false, true] {
        for structure in [WireStructure::Off, WireStructure::Affine, WireStructure::Auto] {
            for floats in [WireFloats::Memcpy, WireFloats::XorDelta] {
                for numerics in [WireNumerics::Varint, WireNumerics::Fixed] {
                    let dials = |f: &dyn Fn() -> Vec<u8>| {
                        with_struct_view(view, || with_structure(structure, || with_floats(floats, || with_numerics(numerics, f))))
                    };
                    let encode = |v: &RuntimeValue| dials(&|| message_to_wire_with("n", v, WireCodec::Native, WireIntegrity::Raw).expect("encodes"));
                    for (k, canon) in columns(f64::NAN).into_iter().enumerate() {
                        for (shape, make) in [("column", &list as &dyn Fn(Vec<f64>) -> RuntimeValue), ("record list", &records)] {
                            let want = encode(&make(canon.clone()));
                            for nan in nans {
                                if encode(&make(columns(nan).swap_remove(k))) != want {
                                    bad.push(format!(
                                        "view {view}, {structure:?}, {floats:?}, {numerics:?}, {shape} {k}: NaN {:#x} encodes differently",
                                        nan.to_bits()
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    for (k, canon) in columns(f64::NAN).into_iter().enumerate() {
        let want = encode_value_raw(&list(canon.clone())).expect("encodes");
        for nan in nans {
            if encode_value_raw(&list(columns(nan).swap_remove(k))).expect("encodes") != want {
                bad.push(format!("the raw value encoding, column {k}: NaN {:#x} encodes differently", nan.to_bits()));
            }
        }
    }
    let complex = |re: f64, im: f64| RuntimeValue::ComplexF(Rc::new(logicaffeine_base::ComplexF::new(re, im)));
    for nan in nans {
        for (re, im) in [(nan, 1.5), (1.5, nan), (nan, nan)] {
            let canon = |x: f64| if x.is_nan() { f64::NAN } else { x };
            if encode_value_raw(&complex(re, im)) != encode_value_raw(&complex(canon(re), canon(im))) {
                bad.push(format!("a complex ({:#x}, {:#x}) encodes a NaN's own bits", re.to_bits(), im.to_bits()));
            }
        }
    }
    for (k, canon) in columns(f64::NAN).into_iter().enumerate() {
        let want = build_columnar_record("n", "T", &[("c", WireColumn::Floats(&canon))]);
        for nan in nans {
            let col = columns(nan).swap_remove(k);
            if build_columnar_record("n", "T", &[("c", WireColumn::Floats(&col))]) != want {
                bad.push(format!("columnar record, column {k}: NaN {:#x} encodes differently", nan.to_bits()));
            }
        }
    }
    assert!(bad.is_empty(), "{} encodings keep a NaN's own bits:\n{}", bad.len(), bad.join("\n"));
}

#[test]
fn the_json_codec_orders_a_map_the_same_whatever_its_nan_keys_carry() {
    use logicaffeine_compile::concurrency::marshal::{message_to_wire_with, WireCodec, WireIntegrity};
    use logicaffeine_compile::interpreter::{MapStorage, RuntimeValue};
    use std::cell::RefCell;
    use std::rc::Rc;
    let map = |keys: [f64; 3]| {
        let mut m = MapStorage::default();
        for (i, k) in keys.into_iter().enumerate() {
            m.insert(RuntimeValue::Float(k), RuntimeValue::Int(i as i64));
        }
        RuntimeValue::Map(Rc::new(RefCell::new(m)))
    };
    let encode = |v: &RuntimeValue| message_to_wire_with("n", v, WireCodec::Json, WireIntegrity::Raw).expect("encodes");
    let want = encode(&map([f64::NAN, 1.5, f64::NAN]));
    for (a, b) in [
        (f64::from_bits(0xFFF8_0000_0000_0000), f64::from_bits(0x7FF8_0000_0000_0001)),
        (f64::from_bits(0x7FF8_0000_0000_0001), f64::from_bits(0xFFF8_0000_0000_0000)),
        (f64::from_bits(u64::MAX), f64::from_bits(0x7FF0_0000_0000_0001)),
    ] {
        assert_eq!(encode(&map([a, 1.5, b])), want, "NaN keys {:#x}, {:#x} order the map by their payloads", a.to_bits(), b.to_bits());
    }
}
