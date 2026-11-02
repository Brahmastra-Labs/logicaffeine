//! G1/G3 mutation-safety lock — idempotency keys + expected-version, PROVED.
//!
//! MaCSad gates real database mutations behind proof that a mutation is safe:
//! a retried request must never apply twice (idempotency key), and a write must
//! be refused when the aggregate has moved past the caller's expected version
//! (optimistic concurrency). The client half already ships in `web.lg`
//! (`postJsonIdempotent` sends `idempotency-key`, `putJsonExpecting` sends
//! `x-expected-version`, and neither retries implicitly). This lock proves the
//! SERVER half is expressible in pure LOGOS with the tools we provide — `headerOf`
//! to read the reserved-free request headers, a store for the applied-key set and
//! the version — by driving the REAL `handle(request) -> Response` on the
//! tree-walker against a hermetic file store, exactly the shape a deployed `Serve`
//! wires. No language change is needed; this pins that the capability is real.
//!
//! Adversarial cases, each a distinct request through the real handler:
//!   * a repeated `idempotency-key` applies the increment EXACTLY ONCE;
//!   * two DIFFERENT keys apply it twice (dedup is by key, not blanket);
//!   * a stale `x-expected-version` PUT is refused 409, value unchanged;
//!   * a current-version PUT succeeds 200 and bumps the version;
//!   * a GET is side-effect free (repeated GETs never change the store).

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::tw_outcome;
use std::path::PathBuf;

/// The store lives at one fixed temp path (the sealed tree-walker `get()` returns
/// None, so the handler's `storePath()` uses its documented default anyway); a
/// mutex serializes the lock so nothing observes a half-written store.
static GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

const STORE_PATH: &str = "/tmp/logos_mutation_safety.txt";

/// The backend, written in pure LOGOS: a versioned counter with idempotent
/// increment and optimistic-concurrency set. The store text is
/// `version\nvalue\n<applied key>\n<applied key>…` — line 1 the version, line 2
/// the value, the rest the set of idempotency keys already applied.
const HANDLER_LG: &str = r#"
## To storePath () -> Text:
    Return "/tmp/logos_mutation_safety.txt".

## To readStore () -> Text:
    Let r be read(storePath()).
    Inspect r:
        When Ok (contents):
            Return contents.
        When Err (e):
            Return "".

## To storeLine (store: Text) and (n: Int) -> Text:
    Let mutable i be 1.
    Repeat for row in lines(store):
        If i is equal to n:
            Return row.
        Set i to i + 1.
    Return "".

## To storeVersion (store: Text) -> Int:
    Let v be storeLine(store, 1).
    If allDigits(v) is equal to true:
        Return parseInt(v).
    Return 0.

## To storeValue (store: Text) -> Int:
    Let v be storeLine(store, 2).
    If allDigits(v) is equal to true:
        Return parseInt(v).
    Return 0.

## To keyApplied (store: Text) and (key: Text) -> Bool:
    Let mutable i be 1.
    Repeat for row in lines(store):
        If i is at least 3:
            If row is equal to key:
                Return true.
        Set i to i + 1.
    Return false.

## To appliedKeys (store: Text) -> Seq of Text:
    Let mutable out be a new Seq of Text.
    Let mutable i be 1.
    Repeat for row in lines(store):
        If i is at least 3:
            If length of text_bytes(row) is greater than 0:
                Push row to out.
        Set i to i + 1.
    Return out.

## To renderStore (version: Int) and (value: Int) and (keys: Seq of Text) -> Text:
    Let mutable out be a new Seq of Text.
    Push "{version}" to out.
    Push "{value}" to out.
    Repeat for k in keys:
        Push k to out.
    Return join(out, "\n").

## To handleIncrement (request: Request) -> Response:
    Let store be readStore().
    Let version be storeVersion(store).
    Let value be storeValue(store).
    Let key be headerOf(request, "idempotency-key").
    If key is equal to "":
        Return a new Response with status 400 and contentType "text/plain" and body "missing idempotency-key".
    If keyApplied(store, key) is equal to true:
        Return plain("{version}|{value}").
    Let amount be parseInt(request's body).
    Let newValue be value + amount.
    Let newVersion be version + 1.
    Let mutable keys be appliedKeys(store).
    Push key to keys.
    Let next be renderStore(newVersion, newValue, keys).
    Let w be write(storePath(), next).
    Return plain("{newVersion}|{newValue}").

## To handleSet (request: Request) -> Response:
    Let store be readStore().
    Let version be storeVersion(store).
    Let value be storeValue(store).
    Let expected be headerOf(request, "x-expected-version").
    If expected is equal to "":
        Return a new Response with status 400 and contentType "text/plain" and body "missing x-expected-version".
    Let expectedVersion be parseInt(expected).
    If expectedVersion is equal to version:
        Let newValue be parseInt(request's body).
        Let newVersion be version + 1.
        Let keys be appliedKeys(store).
        Let next be renderStore(newVersion, newValue, keys).
        Let w be write(storePath(), next).
        Return plain("{newVersion}|{newValue}").
    Return a new Response with status 409 and contentType "text/plain" and body "version conflict".

## To handleRead (request: Request) -> Response:
    Let store be readStore().
    Let version be storeVersion(store).
    Let value be storeValue(store).
    Return plain("{version}|{value}").

## To handle (request: Request) -> Response:
    Let path be request's path.
    Let method be request's method.
    If path is equal to "/counter":
        If method is equal to "GET":
            Return handleRead(request).
    If path is equal to "/counter/increment":
        If method is equal to "POST":
            Return handleIncrement(request).
    If path is equal to "/counter":
        If method is equal to "PUT":
            Return handleSet(request).
    Return notFound().
"#;

/// A LOGOS string literal (escaping `\`, `"`, newlines).
fn lg_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

struct Backend {
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl Backend {
    fn new() -> Self {
        let guard = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("TODO_DB_URL");
        let _ = std::fs::remove_file(STORE_PATH);
        Backend { _guard: guard }
    }

    /// Drive the REAL `handle` with a synthesized `Request` carrying `headers`
    /// (name,value pairs) and return the real `Response`'s `(status, body)`.
    fn call(
        &self,
        method: &str,
        path: &str,
        body: &str,
        headers: &[(&str, &str)],
    ) -> (i64, String) {
        let mut hdr_lines = String::new();
        for (k, v) in headers {
            hdr_lines.push_str(&format!("Set item {} of h to {}.\n", lg_str(k), lg_str(v)));
        }
        let driver = format!(
            "Let mutable h be a new Map of Text to Text.\n\
             {hdr_lines}\
             Let q be a new Map of Text to Text.\n\
             Let req be a new Request with method {} and path {} and body {} and headers h and query q.\n\
             Let resp be handle(req).\n\
             Show \"{{resp's status}}|{{resp's body}}\".",
            lg_str(method),
            lg_str(path),
            lg_str(body),
        );
        let src = format!("{HANDLER_LG}\n## Main\n{driver}");
        let out = tw_outcome(&src);
        assert!(
            out.error.is_none(),
            "the mutation-safety handler errored for {method} {path}:\n{driver}\n\nerror: {:?}",
            out.error
        );
        let line = out.output.trim_end_matches('\n');
        let (status_s, resp_body) = line.split_once('|').unwrap_or(("0", ""));
        (status_s.parse().unwrap_or(0), resp_body.to_string())
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(PathBuf::from(STORE_PATH));
    }
}

#[test]
fn a_repeated_idempotency_key_applies_the_mutation_exactly_once() {
    let be = Backend::new();
    // First increment by 5 under key "k1": applied, version 1, value 5.
    assert_eq!(
        be.call(
            "POST",
            "/counter/increment",
            "5",
            &[("idempotency-key", "k1")]
        ),
        (200, "1|5".into())
    );
    // A RETRY with the SAME key must NOT apply again — value stays 5, version stays 1.
    assert_eq!(
        be.call(
            "POST",
            "/counter/increment",
            "5",
            &[("idempotency-key", "k1")]
        ),
        (200, "1|5".into()),
        "a retried request with the same idempotency-key must never apply twice"
    );
    // A THIRD retry, same key: still once.
    assert_eq!(
        be.call(
            "POST",
            "/counter/increment",
            "5",
            &[("idempotency-key", "k1")]
        ),
        (200, "1|5".into())
    );
    // The store confirms: exactly one increment happened.
    assert_eq!(be.call("GET", "/counter", "", &[]), (200, "1|5".into()));
}

#[test]
fn distinct_idempotency_keys_each_apply() {
    let be = Backend::new();
    assert_eq!(
        be.call(
            "POST",
            "/counter/increment",
            "5",
            &[("idempotency-key", "a")]
        ),
        (200, "1|5".into())
    );
    // A DIFFERENT key is a distinct operation — it applies.
    assert_eq!(
        be.call(
            "POST",
            "/counter/increment",
            "3",
            &[("idempotency-key", "b")]
        ),
        (200, "2|8".into())
    );
    // Re-sending the FIRST key is still deduped even after the second applied.
    assert_eq!(
        be.call(
            "POST",
            "/counter/increment",
            "5",
            &[("idempotency-key", "a")]
        ),
        (200, "2|8".into())
    );
}

#[test]
fn a_missing_idempotency_key_is_refused() {
    let be = Backend::new();
    let (status, body) = be.call("POST", "/counter/increment", "5", &[]);
    assert_eq!(
        status, 400,
        "a mutation without an idempotency-key must be refused, got {body}"
    );
}

#[test]
fn a_stale_expected_version_is_refused_and_a_current_one_succeeds() {
    let be = Backend::new();
    // Establish version 1 (value 5) via an increment.
    assert_eq!(
        be.call(
            "POST",
            "/counter/increment",
            "5",
            &[("idempotency-key", "k1")]
        ),
        (200, "1|5".into())
    );
    // A PUT expecting version 0 (stale) is a 409 conflict — value unchanged.
    assert_eq!(
        be.call("PUT", "/counter", "99", &[("x-expected-version", "0")]),
        (409, "version conflict".into()),
        "a write against a stale expected-version must be refused"
    );
    assert_eq!(
        be.call("GET", "/counter", "", &[]),
        (200, "1|5".into()),
        "the refused write left the value untouched"
    );
    // A PUT expecting the CURRENT version (1) succeeds and bumps to version 2.
    assert_eq!(
        be.call("PUT", "/counter", "99", &[("x-expected-version", "1")]),
        (200, "2|99".into())
    );
    // Re-issuing the SAME expected-version (1) now conflicts — it already moved to 2.
    assert_eq!(
        be.call("PUT", "/counter", "42", &[("x-expected-version", "1")]),
        (409, "version conflict".into())
    );
}

#[test]
fn a_get_is_side_effect_free() {
    let be = Backend::new();
    assert_eq!(
        be.call(
            "POST",
            "/counter/increment",
            "7",
            &[("idempotency-key", "k1")]
        ),
        (200, "1|7".into())
    );
    // Repeated GETs never change the store — same version and value every time.
    for _ in 0..3 {
        assert_eq!(be.call("GET", "/counter", "", &[]), (200, "1|7".into()));
    }
}
