//! The language `Stream` / `Await stream` surface, end-to-end over the loopback relay. A producer
//! batches a list of values into ONE framed stream message (Kafka-style — amortizes per-message
//! overhead); the consumer deframes it back into a list. Both directions are exercised through the
//! running interpreter: `Await stream` (receive) and `Stream … to` (send).

use std::time::Duration;

use logicaffeine_compile::concurrency::marshal::{deframe_stream_message, frame_stream_message};
use logicaffeine_compile::interpret_for_ui;
use logicaffeine_compile::interpreter::RuntimeValue;
use logicaffeine_system::addr::canonical_topic;
use logicaffeine_system::relay::{serve, RelayClient};

#[tokio::test]
async fn await_stream_receives_a_batch_and_reads_an_element() {
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();
    let peer = RelayClient::connect(&url).await.expect("peer dials");

    // The interpreter listens, then `Await stream`s a batch from "sensor" into a list and reads the
    // first element.
    let program = format!(
        "## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Listen on \"me\".\n\
         \x20   Await stream from \"sensor\" into batch.\n\
         \x20   Show item 1 of batch.\n"
    );

    let interp = tokio::time::timeout(Duration::from_secs(10), interpret_for_ui(&program));
    let inject = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let blob = frame_stream_message(
            &canonical_topic("sensor"),
            &[RuntimeValue::Int(11), RuntimeValue::Int(22), RuntimeValue::Int(33)],
        )
        .unwrap();
        peer.publish(&canonical_topic("me"), blob).expect("peer streams the batch");
    };

    let (result, ()) = tokio::join!(interp, inject);
    let result = result.expect("interpreter did not hang");
    assert!(result.error.is_none(), "`Await stream` ran: {:?}", result.error);
    assert!(
        result.lines.iter().any(|l| l == "11"),
        "interpreter deframed the batch and read item 1 (11); output: {:?}",
        result.lines
    );
}

#[tokio::test]
async fn stream_sends_a_batch_that_a_peer_deframes() {
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();
    let mut peer = RelayClient::connect(&url).await.expect("peer dials");
    peer.subscribe(&canonical_topic("sink")).await.expect("peer subscribes to the stream sink");

    // The interpreter batches a list and `Stream`s it to "sink" in one framed message.
    let program = format!(
        "## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Listen on \"me\".\n\
         \x20   Let items be [11, 22, 33].\n\
         \x20   Stream items to \"sink\".\n"
    );

    let interp = tokio::time::timeout(Duration::from_secs(10), interpret_for_ui(&program));
    let recv = async {
        let (_topic, data) = tokio::time::timeout(Duration::from_secs(5), peer.next_event())
            .await
            .expect("the streamed batch arrives in time")
            .expect("event present");
        data
    };

    let (result, data) = tokio::join!(interp, recv);
    let result = result.expect("interpreter did not hang");
    assert!(result.error.is_none(), "`Stream` send ran: {:?}", result.error);

    let values = deframe_stream_message(&data).expect("the peer received a batch stream message");
    assert_eq!(
        values,
        vec![RuntimeValue::Int(11), RuntimeValue::Int(22), RuntimeValue::Int(33)],
        "the peer deframed exactly the values the program streamed"
    );
}
