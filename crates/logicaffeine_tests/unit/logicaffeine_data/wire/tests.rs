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
