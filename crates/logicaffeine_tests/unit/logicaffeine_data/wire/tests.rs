use super::*;

// A mirror of what codegen emits for an algebraic type: an enum with scalar, string,
// bool, float, boxed-recursive, and list fields. If THIS round-trips, the emitted
// impls round-trip.
#[derive(Debug, Clone, PartialEq)]
enum Tree {
    Leaf(i64),
    Str(String),
    Node {
        tag: String,
        flag: bool,
        ratio: f64,
        kids: Vec<Tree>,
        boxed: Box<Tree>,
    },
    Empty,
}

impl WireEncode for Tree {
    fn wire_encode(&self, out: &mut Vec<u8>) {
        match self {
            Tree::Leaf(v) => {
                write_inductive_header(out, "Tree", "Leaf", 1);
                v.wire_encode(out);
            }
            Tree::Str(s) => {
                write_inductive_header(out, "Tree", "Str", 1);
                s.wire_encode(out);
            }
            Tree::Node {
                tag,
                flag,
                ratio,
                kids,
                boxed,
            } => {
                write_inductive_header(out, "Tree", "Node", 5);
                tag.wire_encode(out);
                flag.wire_encode(out);
                ratio.wire_encode(out);
                kids.wire_encode(out);
                boxed.wire_encode(out);
            }
            Tree::Empty => write_inductive_header(out, "Tree", "Empty", 0),
        }
    }
}
impl WireDecode for Tree {
    fn wire_decode(buf: &[u8], pos: &mut usize) -> Option<Self> {
        let (ty, ctor, _n) = read_inductive_header(buf, pos)?;
        assert_eq!(ty, "Tree");
        Some(match ctor.as_str() {
            "Leaf" => Tree::Leaf(i64::wire_decode(buf, pos)?),
            "Str" => Tree::Str(String::wire_decode(buf, pos)?),
            "Node" => Tree::Node {
                tag: String::wire_decode(buf, pos)?,
                flag: bool::wire_decode(buf, pos)?,
                ratio: f64::wire_decode(buf, pos)?,
                kids: Vec::<Tree>::wire_decode(buf, pos)?,
                boxed: Box::<Tree>::wire_decode(buf, pos)?,
            },
            "Empty" => Tree::Empty,
            _ => return None,
        })
    }
}

fn roundtrip(t: &Tree) -> Tree {
    let mut out = Vec::new();
    t.wire_encode(&mut out);
    let mut pos = 0usize;
    let back = Tree::wire_decode(&out, &mut pos).expect("decode");
    assert_eq!(pos, out.len(), "decoder must consume every byte");
    back
}

#[test]
fn varint_roundtrips_across_the_range() {
    for x in [0u64, 1, 127, 128, 16383, 16384, u32::MAX as u64, u64::MAX] {
        let mut b = Vec::new();
        write_uvarint(x, &mut b);
        let mut p = 0;
        assert_eq!(read_uvarint(&b, &mut p), Some(x));
        assert_eq!(p, b.len());
    }
}

#[test]
fn zigzag_roundtrips_including_extremes() {
    for x in [0i64, 1, -1, 42, -42, i64::MAX, i64::MIN] {
        assert_eq!(unzigzag(zigzag(x)), x);
    }
}

#[test]
fn scalar_leaves_roundtrip() {
    for v in [0i64, 1, -1, 42, -99999, i64::MAX, i64::MIN] {
        assert_eq!(roundtrip(&Tree::Leaf(v)), Tree::Leaf(v));
    }
}

#[test]
fn strings_roundtrip_including_empty_and_unicode() {
    for s in [
        "",
        "x",
        "hello world",
        "héllo, 世界! + x_1",
        "\n\t\"quoted\"",
    ] {
        assert_eq!(
            roundtrip(&Tree::Str(s.to_string())),
            Tree::Str(s.to_string())
        );
    }
}

#[test]
fn nested_node_with_every_field_kind_roundtrips() {
    let t = Tree::Node {
        tag: "root".into(),
        flag: true,
        ratio: 3.5,
        kids: vec![
            Tree::Leaf(1),
            Tree::Str("two".into()),
            Tree::Node {
                tag: "inner".into(),
                flag: false,
                ratio: -0.25,
                kids: vec![],
                boxed: Box::new(Tree::Empty),
            },
        ],
        boxed: Box::new(Tree::Leaf(-7)),
    };
    assert_eq!(roundtrip(&t), t);
}

#[test]
fn empty_list_and_nullary_variant_roundtrip() {
    assert_eq!(roundtrip(&Tree::Empty), Tree::Empty);
    let t = Tree::Node {
        tag: "".into(),
        flag: false,
        ratio: 0.0,
        kids: vec![],
        boxed: Box::new(Tree::Empty),
    };
    assert_eq!(roundtrip(&t), t);
}

#[test]
fn deep_recursion_roundtrips() {
    let mut t = Tree::Leaf(0);
    for i in 1..500 {
        t = Tree::Node {
            tag: format!("n{i}"),
            flag: i % 2 == 0,
            ratio: i as f64,
            kids: vec![Tree::Leaf(i)],
            boxed: Box::new(t),
        };
    }
    assert_eq!(roundtrip(&t), t);
}

#[test]
fn logos_seq_roundtrips_and_matches_vec_bytes() {
    use crate::types::LogosSeq;
    let elems = vec![1i64, -2, 3, 0, i64::MAX];
    let seq = LogosSeq::from_vec(elems.clone());

    let mut seq_bytes = Vec::new();
    seq.wire_encode(&mut seq_bytes);
    let mut vec_bytes = Vec::new();
    elems.wire_encode(&mut vec_bytes);
    assert_eq!(
        seq_bytes, vec_bytes,
        "LogosSeq and Vec must share the T_LIST byte format"
    );

    let mut pos = 0usize;
    let back = LogosSeq::<i64>::wire_decode(&seq_bytes, &mut pos).expect("decode");
    assert_eq!(pos, seq_bytes.len());
    assert_eq!(back.to_vec(), elems);

    // empty
    let empty = LogosSeq::<i64>::from_vec(vec![]);
    let mut b = Vec::new();
    empty.wire_encode(&mut b);
    let mut p = 0usize;
    assert_eq!(
        LogosSeq::<i64>::wire_decode(&b, &mut p).unwrap().to_vec(),
        Vec::<i64>::new()
    );
}

#[test]
fn truncated_input_returns_none_never_panics() {
    let mut out = Vec::new();
    Tree::Leaf(123456).wire_encode(&mut out);
    for cut in 0..out.len() {
        let mut pos = 0usize;
        let _ = Tree::wire_decode(&out[..cut], &mut pos); // must not panic
    }
}

#[test]
fn logos_map_roundtrips_in_insertion_order() {
    use crate::types::LogosMap;
    let m: LogosMap<String, i64> = LogosMap::new();
    for (k, v) in [("zeta", 1i64), ("alpha", -7), ("mid", i64::MIN), ("", 0), ("é", i64::MAX)] {
        m.insert(k.to_string(), v);
    }
    let mut bytes = Vec::new();
    m.wire_encode(&mut bytes);
    let mut pos = 0usize;
    let back = LogosMap::<String, i64>::wire_decode(&bytes, &mut pos).expect("decode");
    assert_eq!(pos, bytes.len(), "must consume every byte");
    let entries: Vec<(String, i64)> = back.borrow().iter().map(|(k, v)| (k.clone(), *v)).collect();
    assert_eq!(
        entries,
        vec![
            ("zeta".to_string(), 1),
            ("alpha".to_string(), -7),
            ("mid".to_string(), i64::MIN),
            (String::new(), 0),
            ("é".to_string(), i64::MAX)
        ],
        "a map is its entries in insertion order"
    );
}

#[test]
fn logos_map_bytes_are_the_peer_codecs_map_layout() {
    use crate::types::LogosMap;
    let m: LogosMap<String, bool> = LogosMap::new();
    m.insert("a".to_string(), true);
    m.insert("bc".to_string(), false);
    let mut bytes = Vec::new();
    m.wire_encode(&mut bytes);
    assert_eq!(bytes, vec![T_MAP, 2, T_TEXT, 1, b'a', T_TRUE, T_TEXT, 2, b'b', b'c', T_FALSE]);
    let empty: LogosMap<String, bool> = LogosMap::new();
    let mut b = Vec::new();
    empty.wire_encode(&mut b);
    assert_eq!(b, vec![T_MAP, 0]);
    let mut p = 0usize;
    assert!(LogosMap::<String, bool>::wire_decode(&b, &mut p).expect("decode").is_empty());
}

#[test]
fn a_map_of_enum_values_roundtrips() {
    use crate::types::LogosMap;
    let m: LogosMap<String, Tree> = LogosMap::new();
    m.insert("leaf".to_string(), Tree::Leaf(3));
    m.insert(
        "node".to_string(),
        Tree::Node {
            tag: "t".to_string(),
            flag: false,
            ratio: -0.5,
            kids: vec![Tree::Empty, Tree::Str("s".to_string())],
            boxed: Box::new(Tree::Leaf(-1)),
        },
    );
    let mut bytes = Vec::new();
    m.wire_encode(&mut bytes);
    let mut pos = 0usize;
    let back = LogosMap::<String, Tree>::wire_decode(&bytes, &mut pos).expect("decode");
    assert_eq!(pos, bytes.len());
    let entries: Vec<(String, Tree)> = back.borrow().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let expected: Vec<(String, Tree)> = m.borrow().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    assert_eq!(entries, expected);
}

#[test]
fn a_truncated_map_returns_none_never_panics() {
    use crate::types::LogosMap;
    let m: LogosMap<String, Tree> = LogosMap::new();
    m.insert("k".to_string(), Tree::Leaf(123456));
    m.insert("j".to_string(), Tree::Str("text".to_string()));
    let mut out = Vec::new();
    m.wire_encode(&mut out);
    for cut in 0..out.len() {
        let mut pos = 0usize;
        assert!(
            LogosMap::<String, Tree>::wire_decode(&out[..cut], &mut pos).is_none(),
            "a map cut at byte {cut} of {} decoded",
            out.len()
        );
    }
}
