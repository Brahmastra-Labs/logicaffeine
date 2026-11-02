use super::*;

#[test]
fn default_config_is_the_deployed_shape() {
    let c = LifecycleConfig::default();
    assert_eq!(c.max_inflight, 1024);
    assert_eq!(c.request_timeout, Duration::from_secs(30));
    assert_eq!(c.read_deadline, Duration::from_secs(30));
    assert_eq!(c.drain_timeout, Duration::from_secs(25));
    assert_eq!(c.max_body, MAX_BODY);
    assert_eq!(c.max_response_body, MAX_BODY);
    assert!(
        c.stamp_correlation_id,
        "the lifecycle entry stamps correlation ids by default"
    );
}

#[test]
fn serve_default_is_byte_identical_safe() {
    // The plain-serve config keeps the full lifecycle but drops the
    // correlation-id header, so the cross-tier byte differential is unmoved.
    let c = LifecycleConfig::serve_default();
    assert!(
        !c.stamp_correlation_id,
        "plain serve must NOT stamp a per-process id"
    );
    assert_eq!(c.max_inflight, super::super::server::MAX_INFLIGHT);
}

#[test]
fn not_ready_is_shed_ready_is_admitted() {
    let lc = Lifecycle::new(LifecycleConfig::default());
    assert!(
        matches!(lc.admit(), Admission::NotReady),
        "starts not ready"
    );
    lc.set_ready(true);
    assert!(matches!(lc.admit(), Admission::Admit(_)), "ready → admit");
}

#[test]
fn slots_bound_concurrency_and_free_on_drop() {
    let lc = Lifecycle::new(LifecycleConfig {
        max_inflight: 2,
        ..Default::default()
    });
    lc.set_ready(true);
    let a = lc.admit();
    let b = lc.admit();
    assert!(matches!(a, Admission::Admit(_)));
    assert!(matches!(b, Admission::Admit(_)));
    assert_eq!(lc.inflight(), 2);
    // The 3rd is shed, not queued.
    assert!(
        matches!(lc.admit(), Admission::Overloaded),
        "over cap → 503, promptly"
    );
    drop(a);
    assert_eq!(lc.inflight(), 1);
    assert!(
        matches!(lc.admit(), Admission::Admit(_)),
        "a freed slot admits again"
    );
}

#[test]
fn draining_sheds_even_when_ready() {
    let lc = Lifecycle::new(LifecycleConfig::default());
    lc.set_ready(true);
    assert!(matches!(lc.admit(), Admission::Admit(_)));
    lc.inner.draining.store(true, Ordering::SeqCst);
    assert!(
        matches!(lc.admit(), Admission::NotReady),
        "draining sheds new work"
    );
    assert!(!lc.is_ready());
}

#[test]
fn client_correlation_id_echoed_else_assigned_and_unique() {
    let lc = Lifecycle::new(LifecycleConfig::default());
    assert_eq!(lc.correlation_id(Some("abc123")), "abc123");
    // Blank/whitespace-only client id → assign one.
    assert_ne!(lc.correlation_id(Some("   ")), "   ");
    // Two no-id requests get DISTINCT ids.
    let a = lc.correlation_id(None);
    let b = lc.correlation_id(None);
    assert_ne!(a, b, "assigned ids must be unique");
    assert!(!a.is_empty() && !b.is_empty());
}

#[test]
fn stamp_replaces_any_casing_and_owns_the_header() {
    let mut resp = service_unavailable();
    resp.headers
        .insert("x-correlation-id".to_string(), "handler-set".to_string());
    stamp_correlation_id(&mut resp, "authoritative");
    // Exactly one correlation header, canonical casing, our value.
    let n = resp
        .headers
        .keys()
        .filter(|k| k.eq_ignore_ascii_case(CORRELATION_HEADER))
        .count();
    assert_eq!(
        n, 1,
        "the lifecycle owns the correlation header (no duplicate)"
    );
    assert_eq!(
        resp.headers.get(CORRELATION_HEADER).map(String::as_str),
        Some("authoritative")
    );
}

#[test]
fn stamp_sanitizes_crlf_so_a_correlation_id_can_never_split_the_response() {
    let mut resp = service_unavailable();
    stamp_correlation_id(&mut resp, "abc\r\nSet-Cookie: pwned\0");
    let v = resp
        .headers
        .get(CORRELATION_HEADER)
        .map(String::as_str)
        .unwrap();
    assert!(!v.contains('\r') && !v.contains('\n') && !v.contains('\0'));
    assert_eq!(
        v, "abcSet-Cookie: pwned",
        "control bytes stripped, rest preserved"
    );
}

#[test]
fn access_log_is_secret_safe() {
    // The line carries method/path/status/id/duration — and structurally
    // cannot carry an arbitrary request header value.
    let line = access_log_line("GET", "/x", 200, "cid-1", Duration::from_millis(4));
    assert!(line.contains("method=GET"));
    assert!(line.contains("path=/x"));
    assert!(line.contains("status=200"));
    assert!(line.contains("correlation_id=cid-1"));
    assert!(line.contains("dur_ms=4"));
    assert!(!line.to_lowercase().contains("authorization"));
    assert!(!line.to_lowercase().contains("cookie"));
}

#[test]
fn service_unavailable_is_a_503() {
    assert_eq!(service_unavailable().status, 503);
}

// ── The environment seam: the one place every tier reads its knobs from. ──

fn env_of(pairs: &[(&str, &str)]) -> LifecycleConfig {
    let owned: Vec<(String, String)> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    LifecycleConfig::from_env_with(move |k| {
        owned.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())
    })
}

#[test]
fn an_empty_environment_is_exactly_the_serve_default() {
    let c = env_of(&[]);
    let d = LifecycleConfig::serve_default();
    assert_eq!(c.max_inflight, d.max_inflight);
    assert_eq!(c.request_timeout, d.request_timeout);
    assert_eq!(c.read_deadline, d.read_deadline);
    assert_eq!(c.drain_timeout, d.drain_timeout);
    assert_eq!(c.max_body, d.max_body);
    assert!(
        !c.stamp_correlation_id,
        "correlation ids stay off by default"
    );
    assert!(
        c.ready_path.is_none(),
        "no path is reserved unless one is named"
    );
    assert!(c.health_path.is_none());
}

#[test]
fn each_knob_is_read_from_its_own_name() {
    let c = env_of(&[
        ("LOGOS_MAX_INFLIGHT", "7"),
        ("LOGOS_REQUEST_TIMEOUT_MS", "1500"),
        ("LOGOS_READ_DEADLINE_MS", "900"),
        ("LOGOS_DRAIN_MS", "250"),
        ("LOGOS_MAX_BODY", "4096"),
        ("LOGOS_CORRELATION_ID", "1"),
        ("LOGOS_READY_PATH", "/readyz"),
        ("LOGOS_HEALTH_PATH", "/healthz"),
    ]);
    assert_eq!(c.max_inflight, 7);
    assert_eq!(c.request_timeout, Duration::from_millis(1500));
    assert_eq!(c.read_deadline, Duration::from_millis(900));
    assert_eq!(c.drain_timeout, Duration::from_millis(250));
    assert_eq!(c.max_body, 4096);
    assert!(c.stamp_correlation_id);
    assert_eq!(c.ready_path.as_deref(), Some("/readyz"));
    assert_eq!(c.health_path.as_deref(), Some("/healthz"));
}

#[test]
fn a_nonsense_value_is_ignored_rather_than_fatal() {
    // A typo in a deployment's environment must never stop a server booting.
    let c = env_of(&[
        ("LOGOS_MAX_INFLIGHT", "lots"),
        ("LOGOS_REQUEST_TIMEOUT_MS", ""),
        ("LOGOS_MAX_BODY", "0"),
        ("LOGOS_READY_PATH", "readyz"),
    ]);
    let d = LifecycleConfig::serve_default();
    assert_eq!(c.max_inflight, d.max_inflight);
    assert_eq!(c.request_timeout, d.request_timeout);
    assert_eq!(
        c.max_body, d.max_body,
        "a zero body cap would refuse every request"
    );
    assert!(
        c.ready_path.is_none(),
        "a probe path must be rooted to be honored"
    );
}

#[test]
fn only_an_affirmative_word_turns_correlation_ids_on() {
    for yes in ["1", "true", "TRUE", "yes", "on"] {
        assert!(
            env_of(&[("LOGOS_CORRELATION_ID", yes)]).stamp_correlation_id,
            "{yes}"
        );
    }
    for no in ["0", "false", "no", "off", "", "maybe"] {
        assert!(
            !env_of(&[("LOGOS_CORRELATION_ID", no)]).stamp_correlation_id,
            "{no}"
        );
    }
}

// ── The probes: opt-in, answered ahead of admission, readiness ≠ liveness. ──

fn probing_lifecycle() -> Lifecycle {
    let mut cfg = LifecycleConfig::serve_default();
    cfg.ready_path = Some("/readyz".to_string());
    cfg.health_path = Some("/healthz".to_string());
    let l = Lifecycle::new(cfg);
    l.set_ready(true);
    l
}

#[test]
fn no_path_is_reserved_when_none_is_configured() {
    let l = Lifecycle::new(LifecycleConfig::serve_default());
    l.set_ready(true);
    for path in ["/readyz", "/healthz", "/", "/api/things"] {
        assert!(
            l.probe_response(path).is_none(),
            "{path} must belong to the program when no probe is configured"
        );
    }
}

#[test]
fn a_configured_probe_answers_only_its_own_path() {
    let l = probing_lifecycle();
    assert_eq!(l.probe_response("/readyz").map(|r| r.status), Some(200));
    assert_eq!(l.probe_response("/healthz").map(|r| r.status), Some(200));
    assert!(l.probe_response("/readyz/deeper").is_none());
    assert!(l.probe_response("/").is_none());
}

#[tokio::test]
async fn draining_makes_the_server_unready_while_it_stays_alive() {
    // The whole point of two probes: an orchestrator must stop SENDING traffic
    // to a draining server without KILLING it, or the requests it is finishing
    // are thrown away.
    let l = probing_lifecycle();
    assert_eq!(l.probe_response("/readyz").map(|r| r.status), Some(200));
    l.drain(Duration::from_millis(10)).await;
    assert_eq!(
        l.probe_response("/readyz").map(|r| r.status),
        Some(503),
        "a draining server must stop advertising readiness"
    );
    assert_eq!(
        l.probe_response("/healthz").map(|r| r.status),
        Some(200),
        "a draining server is still alive and must not be killed"
    );
}
