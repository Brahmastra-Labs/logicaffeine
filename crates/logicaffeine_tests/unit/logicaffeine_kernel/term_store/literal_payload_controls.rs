use super::*;

fn literal(store: &mut TermStore, value: Literal) -> LitId {
    let id = store.intern(&Term::Lit(value));
    let Node::Lit(lit) = *store.node(id) else {
        panic!("literal node")
    };
    lit
}

#[test]
fn text_payload_counts_utf8_bytes_without_extracting_text() {
    let mut store = TermStore::new();
    let empty = literal(&mut store, Literal::Text(String::new()));
    let text = literal(&mut store, Literal::Text("aλ😀".into()));
    assert_eq!(store.literal_payload_bytes(empty), 0);
    assert_eq!(store.literal_payload_bytes(text), 7);
    let before = store.len();
    for _ in 0..100 {
        assert_eq!(store.literal_payload_bytes(text), 7);
    }
    assert_eq!(store.len(), before);
}

#[test]
fn arbitrary_integer_payload_preserves_nat_and_signed_integer_magnitudes() {
    let mut store = TermStore::new();
    let mut bytes = vec![0; 513];
    bytes[512] = 1;
    let positive = BigInt::from_le_bytes(false, &bytes);
    let negative = BigInt::from_le_bytes(true, &bytes);
    let nat = literal(&mut store, Literal::Nat(positive));
    let signed = literal(&mut store, Literal::BigInt(negative));
    // 2^4096 occupies 65 64-bit limbs; the sign is in the fixed header.
    assert_eq!(store.literal_payload_bytes(nat), 520);
    assert_eq!(store.literal_payload_bytes(signed), 520);
}

#[test]
fn inline_numbers_and_other_fixed_literals_need_no_heap_payload_copy() {
    let mut store = TermStore::new();
    for value in [
        Literal::Int(-1),
        Literal::Float(f64::NAN),
        Literal::Float(-0.0),
        Literal::Duration(7),
        Literal::Date(0),
        Literal::Moment(123),
        Literal::Nat(BigInt::zero()),
        Literal::BigInt(BigInt::from_i64(-7)),
    ] {
        let id = literal(&mut store, value);
        assert_eq!(store.literal_payload_bytes(id), 0);
    }
}
