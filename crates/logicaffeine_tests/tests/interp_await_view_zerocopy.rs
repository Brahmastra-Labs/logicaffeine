//! The zero-copy receive, end-to-end through the RUNNING interpreter (capnp's home: "no decode in
//! production"). A peer publishes a self-describing record list over the loopback relay; an
//! interpreted program does `Await view from "sensor" into rows` — which holds the frame lazily
//! (`ListRepr::WireStructs`, no rows decoded) — then reads the first row's field in place. The
//! decode primitives are unit-tested in `concurrency::marshal::tests`; this proves the whole knob →
//! drain → lazy-bind → read chain runs live.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use logicaffeine_compile::concurrency::marshal::{
    message_to_wire_with, with_struct_view, WireCodec, WireIntegrity,
};
use logicaffeine_compile::interpret_for_ui;
use logicaffeine_compile::interpreter::{ListRepr, RuntimeValue, StructValue};
use logicaffeine_system::addr::canonical_topic;
use logicaffeine_system::relay::{serve, RelayClient};

fn reading(score: i64) -> RuntimeValue {
    let mut fields = HashMap::new();
    fields.insert("score".to_string(), RuntimeValue::Int(score));
    RuntimeValue::Struct(Box::new(StructValue { type_name: "Reading".to_string(), fields }))
}

#[tokio::test]
async fn await_view_receives_a_record_list_and_reads_a_field_zero_copy() {
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();
    let peer = RelayClient::connect(&url).await.expect("peer dials");

    // The interpreter dials, listens, then awaits a record list from "sensor" with the `view` knob
    // (zero-copy / decode-on-touch) and reads the first row's `score` field.
    let program = format!(
        "## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Listen on \"me\".\n\
         \x20   Await view from \"sensor\" into rows.\n\
         \x20   Let first be item 1 of rows.\n\
         \x20   Show first's score.\n"
    );

    let interp = tokio::time::timeout(Duration::from_secs(10), interpret_for_ui(&program));
    let inject = async {
        // Let the interpreter subscribe (Connect + Listen), then publish the record list from
        // "sensor" to the interpreter's inbox ("me"), encoded as a `T_STRUCTS_VIEW` record-list view.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let list = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(vec![
            reading(11),
            reading(22),
            reading(33),
        ]))));
        let bytes = with_struct_view(true, || {
            message_to_wire_with(&canonical_topic("sensor"), &list, WireCodec::Native, WireIntegrity::Raw).unwrap()
        });
        peer.publish(&canonical_topic("me"), bytes).expect("peer publishes the record list");
    };

    let (result, ()) = tokio::join!(interp, inject);
    let result = result.expect("interpreter did not hang");

    assert!(result.error.is_none(), "`Await view` ran without error: {:?}", result.error);
    assert!(
        result.lines.iter().any(|l| l == "11"),
        "the interpreter read the first row's score (11) from the zero-copy received list; output: {:?}",
        result.lines
    );
}
