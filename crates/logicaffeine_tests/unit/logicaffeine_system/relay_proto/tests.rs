use super::*;

#[test]
fn frames_round_trip_through_bytes() {
    for frame in [
        RelayFrame::Subscribe {
            topic: "chat".into(),
        },
        RelayFrame::Unsubscribe {
            topic: "chat".into(),
        },
        RelayFrame::Publish {
            topic: "chat".into(),
            data: vec![1, 2, 3],
        },
        RelayFrame::SubAck {
            topic: "chat".into(),
        },
        RelayFrame::Event {
            topic: "chat".into(),
            data: vec![9],
        },
    ] {
        let bytes = frame.to_bytes();
        assert_eq!(RelayFrame::from_bytes(&bytes), Some(frame));
    }
}
