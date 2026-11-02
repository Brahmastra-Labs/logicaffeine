use super::*;

fn assert_send<T: Send>() {}

#[test]
fn rtpayload_is_send() {
    // Compile-time guarantee: payloads can cross thread boundaries. This is
    // the property the M:N work-stealing driver depends on (only `RtPayload`
    // and small ids ever cross a worker boundary).
    assert_send::<RtPayload>();
}

#[test]
fn rtpayload_roundtrips_structurally() {
    let v = RtPayload::Struct {
        type_name: "Point".into(),
        fields: vec![
            ("x".into(), RtPayload::Int(1)),
            (
                "y".into(),
                RtPayload::List(vec![RtPayload::Bool(true), RtPayload::Text("hi".into())]),
            ),
        ],
    };
    assert_eq!(v.clone(), v);
}

#[test]
fn rtpayload_nested_collections() {
    let m = RtPayload::Map(vec![
        (RtPayload::Text("a".into()), RtPayload::Int(1)),
        (
            RtPayload::Text("b".into()),
            RtPayload::Set(vec![RtPayload::Char('x')]),
        ),
    ]);
    match m {
        RtPayload::Map(entries) => assert_eq!(entries.len(), 2),
        _ => panic!("expected map"),
    }
}
