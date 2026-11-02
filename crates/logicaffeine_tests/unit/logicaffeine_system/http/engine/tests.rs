use super::*;
use std::collections::BTreeMap;

fn probe_ok() -> Result<(), String> {
    Ok(())
}
fn probe_fail() -> Result<(), String> {
    Err("no ring".to_string())
}

#[test]
fn a_configured_lifecycle_guarantee_is_never_silently_dropped() {
    // Each of these names asks for something the uring engine cannot enforce.
    // Serving anyway would mean a deployment that configured a request budget,
    // a probe path or bounded backpressure quietly got none of it.
    for knob in [
        "LOGOS_REQUEST_TIMEOUT_MS",
        "LOGOS_MAX_INFLIGHT",
        "LOGOS_READY_PATH",
        "LOGOS_HEALTH_PATH",
        "LOGOS_CORRELATION_ID",
    ] {
        let found = lifecycle_knob_requested(|k| (k == knob).then(|| "x".to_string()));
        assert_eq!(found, Some(knob), "{knob} must force the tokio engine");
    }
}

#[test]
fn the_caps_uring_does_enforce_do_not_cost_the_fast_engine() {
    // Body and read-deadline caps are properties of PARSING, which the uring
    // engine shares verbatim — configuring them must not demote the engine.
    for knob in ["LOGOS_MAX_BODY", "LOGOS_READ_DEADLINE_MS", "LOGOS_DRAIN_MS"] {
        let found = lifecycle_knob_requested(|k| (k == knob).then(|| "x".to_string()));
        assert_eq!(found, None, "{knob} must not demote the engine");
    }
    assert_eq!(
        lifecycle_knob_requested(|_| None),
        None,
        "an empty environment keeps uring"
    );
}

#[test]
fn selection_law_covers_every_knob_setting() {
    // Explicit tokio always wins, no probe consulted.
    let c = choose_engine_from(Some("tokio"), false, || panic!("probe must not run"));
    assert_eq!(c.kind, EngineKind::Tokio);

    // Explicit uring with a working ring.
    let c = choose_engine_from(Some("uring"), false, probe_ok);
    assert_eq!(c.kind, EngineKind::Uring);
    assert!(c.reason.contains("LOGOS_HTTP_ENGINE=uring"));

    // Explicit uring but the ring cannot initialize → tokio, with the why.
    let c = choose_engine_from(Some("uring"), false, probe_fail);
    assert_eq!(c.kind, EngineKind::Tokio);
    assert!(c.reason.contains("no ring"), "reason: {:?}", c.reason);

    // Forced fallback beats a working ring — and names the knob.
    let c = choose_engine_from(Some("uring"), true, probe_ok);
    assert_eq!(c.kind, EngineKind::Tokio);
    assert!(
        c.reason.contains("LOGOS_HTTP_FORCE_FALLBACK"),
        "reason: {:?}",
        c.reason
    );

    // Default (no knob): uring where the ring initializes, else tokio.
    assert_eq!(
        choose_engine_from(None, false, probe_ok).kind,
        EngineKind::Uring
    );
    assert_eq!(
        choose_engine_from(None, false, probe_fail).kind,
        EngineKind::Tokio
    );

    // Whitespace and case are tolerated; garbage falls back loudly.
    assert_eq!(
        choose_engine_from(Some(" URING "), false, probe_ok).kind,
        EngineKind::Uring
    );
    assert_eq!(
        choose_engine_from(Some("Tokio"), false, probe_ok).kind,
        EngineKind::Tokio
    );
    let c = choose_engine_from(Some("hyper"), false, probe_ok);
    assert_eq!(c.kind, EngineKind::Tokio);
    assert!(
        c.reason.contains("not recognized"),
        "reason: {:?}",
        c.reason
    );
}

#[test]
fn shard_count_law() {
    assert_eq!(shard_count_from(None, 8), 8);
    assert_eq!(shard_count_from(Some("4"), 8), 4);
    assert_eq!(shard_count_from(Some(" 2 "), 8), 2);
    assert_eq!(shard_count_from(Some("0"), 8), 8); // zero is not a shard count
    assert_eq!(shard_count_from(Some("nope"), 8), 8);
    assert_eq!(shard_count_from(Some("99999"), 8), 1024); // clamped
    assert_eq!(shard_count_from(None, 0), 1); // parallelism probe failed
}

fn resp(body: &str) -> HttpResponse {
    let mut headers = BTreeMap::new();
    headers.insert("Content-Type".to_string(), "text/html".to_string());
    HttpResponse {
        status: 200,
        headers,
        body: body.as_bytes().to_vec(),
    }
}

fn get(path: &str, query: &[(&str, &str)]) -> HttpRequest {
    HttpRequest {
        method: "GET".to_string(),
        path: path.to_string(),
        query: query
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        headers: BTreeMap::new(),
        body: Vec::new(),
    }
}

#[test]
fn static_table_bytes_are_exactly_the_serializers_bytes() {
    let mut table = StaticResponses::new();
    table.insert("/", &resp("<h1>home</h1>"));
    let req = get("/", &[]);
    assert_eq!(
        table.wire_frame(&req, true).expect("keep-alive variant"),
        serialize_response_conn(&resp("<h1>home</h1>"), true).as_slice()
    );
    assert_eq!(
        table.wire_frame(&req, false).expect("close variant"),
        serialize_response_conn(&resp("<h1>home</h1>"), false).as_slice()
    );
}

#[test]
fn static_table_matches_only_bare_gets_on_known_paths() {
    let mut table = StaticResponses::new();
    table.insert("/", &resp("home"));
    assert!(table.wire_frame(&get("/", &[]), true).is_some());
    assert!(
        table.wire_frame(&get("/missing", &[]), true).is_none(),
        "unknown path"
    );
    assert!(
        table.wire_frame(&get("/", &[("a", "1")]), true).is_none(),
        "query present"
    );
    let mut post = get("/", &[]);
    post.method = "POST".to_string();
    assert!(table.wire_frame(&post, true).is_none(), "non-GET");
    assert!(StaticResponses::new().is_empty());
    assert_eq!(table.len(), 1);
}
