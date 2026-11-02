//! JSON typed structured-decoding lock — the G11 remainder of the pure-LOGOS
//! stdlib json module (`crates/logicaffeine_compile/assets/std/json.lg`),
//! held to cross-tier agreement (tree-walker ≡ VM) on every case.
//!
//! Two contracts sit on top of the RFC 8259 parser locked by
//! `json_conformance_lock`:
//!
//! 1. **Strict decode REJECTS duplicate object keys.** The lenient `jsonTryParse`
//!    keeps last-write-wins (JS `JSON.parse` compatible, its own lock owns that);
//!    the new strict entry point `jsonParseStrict` REJECTS `{"a":1,"a":2}` with a
//!    positioned `JErr`, recursively (a duplicate nested anywhere is caught). No
//!    authorization or money decision is ever made on a document that silently
//!    dropped a colliding key.
//!
//! 2. **Total, typed extractors.** `jsonField` / `jsonString` / `jsonInt` /
//!    `jsonBool` / `jsonArray` / `jsonRequireObject` / `jsonNoExtraKeys` each
//!    return a `Result` (`Ok value` / `Err message`): a missing field, a
//!    wrong-typed field, an extra key, or a bad enum tag REJECTS — never silently
//!    defaults. A caller composes them to decode a command envelope into typed
//!    fields with a tagged-enum `kind`, so "no decision on an untyped field" is
//!    forced by construction.

mod common;
use common::{assert_all_tiers, run_all_tiers};

/// The tree-walker's recursive descent is Rust-stack-hungry per interpreted call;
/// the deep-nesting case runs on a dedicated big-stack thread (the established
/// idiom, mirrored from `json_conformance_lock`).
fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("test thread panicked")
}

/// Render a JSON document as a LOGOS Text expression, byte-exactly, sidestepping
/// LOGOS string-literal escaping/interpolation entirely (the same technique
/// `json_conformance_lock` uses).
fn lg_text(s: &str) -> String {
    if s.is_empty() {
        return "\"\"".to_string();
    }
    let bytes: Vec<String> = s.bytes().map(|b| b.to_string()).collect();
    format!("text_from_bytes([{}])", bytes.join(", "))
}

/// A program that STRICT-parses `json` and Shows either the canonical
/// serialization (`JOk`) or `REJECTED: <msg>` (`JErr`).
fn strict_render_src(json: &str) -> String {
    format!(
        "## Main\n\
         Let s be {}.\n\
         Inspect jsonParseStrict(s):\n\
         \x20   When JOk (v):\n\
         \x20       Show jsonSerialize(v).\n\
         \x20   When JErr (msg):\n\
         \x20       Let bad be \"REJECTED: \" combined with msg.\n\
         \x20       Show bad.\n",
        lg_text(json)
    )
}

/// The STRICT parser must ACCEPT `json` and serialize canonically to `canonical`,
/// on all tiers.
fn strict_accepts(json: &str, canonical: &str) {
    let src = strict_render_src(json);
    let agreed = run_all_tiers(&src);
    assert!(
        agreed.error.is_none(),
        "strict accept case {:?} must run cleanly, got error: {:?}",
        json,
        agreed.error
    );
    assert_eq!(
        agreed.output.trim(),
        canonical,
        "strict accept case {:?}: canonical serialization mismatch",
        json
    );
}

/// The STRICT parser must REJECT `json` via the module error form — `JErr` — with
/// no runtime trap and no wrong success.
fn strict_rejects(json: &str) {
    let src = strict_render_src(json);
    let agreed = run_all_tiers(&src);
    assert!(
        agreed.error.is_none(),
        "strict reject case {:?} must produce JErr, NOT a runtime trap: {:?}",
        json,
        agreed.error
    );
    assert!(
        agreed.output.trim().starts_with("REJECTED: "),
        "strict reject case {:?} was wrongly ACCEPTED as: {}",
        json,
        agreed.output.trim()
    );
}

/// Like [`strict_rejects`], but pins the exact positioned diagnostic.
fn strict_rejects_with(json: &str, msg: &str) {
    let src = strict_render_src(json);
    let agreed = run_all_tiers(&src);
    assert!(
        agreed.error.is_none(),
        "strict reject case {:?} must produce JErr, NOT a runtime trap: {:?}",
        json,
        agreed.error
    );
    assert_eq!(
        agreed.output.trim(),
        format!("REJECTED: {}", msg),
        "strict reject case {:?}: error message contract mismatch",
        json
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Contract 1 — duplicate object keys are REJECTED by the strict decode path.
// ─────────────────────────────────────────────────────────────────────────────

/// The exact document the lenient parser silently collapses is now caught with a
/// positioned message (the byte of the offending SECOND key's opening quote).
#[test]
fn strict_rejects_duplicate_object_keys() {
    strict_rejects(r#"{"a":1,"a":2}"#);
    strict_rejects(r#"{"a":1,"b":2,"a":3}"#);
    strict_rejects(r#"{"x":1,"y":2,"z":3,"x":4}"#);
}

/// A duplicate nested ANYWHERE in the tree is caught — strictness is recursive,
/// not just top-level.
#[test]
fn strict_rejects_nested_duplicate_object_keys() {
    strict_rejects(r#"{"outer":{"a":1,"a":2}}"#);
    strict_rejects(r#"[1,{"k":true,"k":false}]"#);
    strict_rejects(r#"{"a":{"b":{"c":1,"c":2}}}"#);
}

/// The diagnostic is specific and positioned — 1-based byte of the second key.
#[test]
fn strict_duplicate_message_is_positioned() {
    // {"a":1,"a":2}
    //  1234567890...  — the second "a" opens at byte 8.
    strict_rejects_with(r#"{"a":1,"a":2}"#, "duplicate object key \"a\" at byte 8");
}

/// A document with NO duplicates strict-parses IDENTICALLY to the lenient parser —
/// strictness only adds a rejection, it never changes an accepted value. Every
/// accept here is also a serialization fixed point.
#[test]
fn strict_accepts_unique_keyed_documents() {
    strict_accepts(r#"{"a":1,"b":2,"c":3}"#, r#"{"a":1,"b":2,"c":3}"#);
    strict_accepts(
        r#"{"a":{"b":1},"c":{"b":2}}"#,
        r#"{"a":{"b":1},"c":{"b":2}}"#,
    );
    strict_accepts(r#"[{"a":1},{"a":2}]"#, r#"[{"a":1},{"a":2}]"#);
    strict_accepts("{}", "{}");
    strict_accepts(r#"{"a":[1,2,3],"b":"x"}"#, r#"{"a":[1,2,3],"b":"x"}"#);
    strict_accepts("42", "42");
    strict_accepts(r#"["a","b","c"]"#, r#"["a","b","c"]"#);
}

/// Strict parse still enforces EVERY RFC rejection the lenient parser does — the
/// duplicate rule is ADDED, the rest is inherited unchanged.
#[test]
fn strict_still_rejects_ordinary_malformations() {
    strict_rejects("[1,");
    strict_rejects("{}extra");
    strict_rejects(r#"{"a":}"#);
    strict_rejects("9223372036854775808");
    strict_rejects("tru");
}

/// The lenient `jsonTryParse` is UNCHANGED: `{"a":1,"a":2}` still last-write-wins.
/// This pins that the strict path did NOT weaken the lenient one (which
/// `json_on_wasm_lock`/the todo capstone depend on).
#[test]
fn lenient_try_parse_still_last_write_wins() {
    let src = format!(
        "## Main\n\
         Let s be {}.\n\
         Inspect jsonTryParse(s):\n\
         \x20   When JOk (v):\n\
         \x20       Show jsonSerialize(v).\n\
         \x20   When JErr (msg):\n\
         \x20       Show msg.\n",
        lg_text(r#"{"a":1,"a":2}"#)
    );
    assert_all_tiers(&src, r#"{"a":2}"#);
}

// ─────────────────────────────────────────────────────────────────────────────
// Contract 2 — total, typed extractors + a decoded command envelope.
// ─────────────────────────────────────────────────────────────────────────────

/// `jsonString` / `jsonInt` / `jsonBool` return `Ok payload` on the right type and
/// `Err message` on the wrong one — never a silent default.
#[test]
fn typed_scalar_extractors_accept_right_type_reject_wrong() {
    let src = "## Main\n\
        Let sv be a new JStr with s \"hi\".\n\
        Let iv be a new JNum with n 7.\n\
        Let bv be a new JBool with b true.\n\
        Inspect jsonString(sv):\n\
        \x20   When Ok (t):\n\
        \x20       Show t.\n\
        \x20   When Err (e):\n\
        \x20       Show e.\n\
        Inspect jsonInt(iv):\n\
        \x20   When Ok (n):\n\
        \x20       Show n.\n\
        \x20   When Err (e):\n\
        \x20       Show e.\n\
        Inspect jsonBool(bv):\n\
        \x20   When Ok (b):\n\
        \x20       Show b.\n\
        \x20   When Err (e):\n\
        \x20       Show e.\n\
        Inspect jsonString(iv):\n\
        \x20   When Ok (t):\n\
        \x20       Show t.\n\
        \x20   When Err (e):\n\
        \x20       Show e.\n\
        Inspect jsonInt(sv):\n\
        \x20   When Ok (n):\n\
        \x20       Show n.\n\
        \x20   When Err (e):\n\
        \x20       Show e.\n\
        Inspect jsonBool(iv):\n\
        \x20   When Ok (b):\n\
        \x20       Show b.\n\
        \x20   When Err (e):\n\
        \x20       Show e.\n";
    assert_all_tiers(
        src,
        "hi\n7\ntrue\n\
         expected a JSON string\n\
         expected a JSON integer\n\
         expected a JSON boolean",
    );
}

/// `jsonField` returns `Ok val` when the key is present and `Err` when it is
/// missing — the missing case is a REJECTION, not a `JNull`.
#[test]
fn json_field_present_and_missing() {
    let src = format!(
        "## Main\n\
         Let v be jsonParse({}).\n\
         Inspect jsonField(v, \"a\"):\n\
         \x20   When Ok (fv):\n\
         \x20       Show jsonSerialize(fv).\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Inspect jsonField(v, \"missing\"):\n\
         \x20   When Ok (fv):\n\
         \x20       Show jsonSerialize(fv).\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n",
        lg_text(r#"{"a":1}"#)
    );
    assert_all_tiers(&src, "1\nmissing field \"missing\"");
}

/// `jsonArray` returns `Ok elems` on an array and `Err` otherwise;
/// `jsonRequireObject` returns `Ok entries` on an object and `Err` otherwise.
#[test]
fn json_array_and_require_object() {
    let src = format!(
        "## Main\n\
         Let arr be jsonParse(\"[1,2,3]\").\n\
         Inspect jsonArray(arr):\n\
         \x20   When Ok (xs):\n\
         \x20       Show length of xs.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Inspect jsonArray(jsonParse(\"7\")):\n\
         \x20   When Ok (xs):\n\
         \x20       Show length of xs.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Let obj be jsonParse({}).\n\
         Inspect jsonRequireObject(obj):\n\
         \x20   When Ok (es):\n\
         \x20       Show length of es.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Inspect jsonRequireObject(arr):\n\
         \x20   When Ok (es):\n\
         \x20       Show length of es.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n",
        lg_text(r#"{"a":1,"b":2}"#)
    );
    assert_all_tiers(&src, "3\nexpected a JSON array\n2\nexpected a JSON object");
}

/// `jsonNoExtraKeys(obj, allowed)` returns `Ok true` when every key is in the
/// allow-list and `Err "unexpected key …"` on the first key that is not.
#[test]
fn json_no_extra_keys_accepts_subset_rejects_unexpected() {
    let allowed = "Let allowed be a new Seq of Text.\n\
         Push \"kind\" to allowed.\n\
         Push \"text\" to allowed.\n\
         Push \"version\" to allowed.\n";
    let ok_src = format!(
        "## Main\n\
         {allowed}\
         Let obj be jsonParse({}).\n\
         Inspect jsonNoExtraKeys(obj, allowed):\n\
         \x20   When Ok (b):\n\
         \x20       Show \"OK\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n",
        lg_text(r#"{"kind":"add","text":"milk"}"#)
    );
    assert_all_tiers(&ok_src, "OK");

    let bad_src = format!(
        "## Main\n\
         {allowed}\
         Let obj be jsonParse({}).\n\
         Inspect jsonNoExtraKeys(obj, allowed):\n\
         \x20   When Ok (b):\n\
         \x20       Show \"OK\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n",
        lg_text(r#"{"kind":"add","surprise":1}"#)
    );
    assert_all_tiers(&bad_src, "unexpected key \"surprise\"");
}

/// The capstone: a full command-envelope decode. `{"kind":"add","text":"milk",
/// "version":3}` decodes into a typed `kind` (tagged enum), `text` (Text), and
/// `version` (Int); a VALID envelope decodes, and a missing field, a wrong-typed
/// field, an extra key, and a BAD enum tag each REJECT. The decoders force a typed,
/// validated read — no decision is ever taken on an untyped field.
///
/// `decodeCommand` returns the tag name on success (`"ADD"`/`"REMOVE"`) or a
/// diagnostic on any rejection; it lives in the test as a small composed decoder
/// over the library extractors, exactly as a real caller composes them.
fn decode_command_program(json: &str) -> String {
    format!(
        "## To decodeCommand (doc: Text) -> Text:\n\
         \x20   Let pj be jsonParseStrict(doc).\n\
         \x20   Inspect pj:\n\
         \x20       When JErr (m):\n\
         \x20           Return \"parse: \" combined with m.\n\
         \x20       When JOk (v):\n\
         \x20           Let allowed be a new Seq of Text.\n\
         \x20           Push \"kind\" to allowed.\n\
         \x20           Push \"text\" to allowed.\n\
         \x20           Push \"version\" to allowed.\n\
         \x20           Inspect jsonNoExtraKeys(v, allowed):\n\
         \x20               When Err (e):\n\
         \x20                   Return e.\n\
         \x20               When Ok (b0):\n\
         \x20                   Inspect jsonField(v, \"kind\"):\n\
         \x20                       When Err (e):\n\
         \x20                           Return e.\n\
         \x20                       When Ok (kf):\n\
         \x20                           Inspect jsonString(kf):\n\
         \x20                               When Err (e):\n\
         \x20                                   Return e.\n\
         \x20                               When Ok (ks):\n\
         \x20                                   Inspect jsonField(v, \"text\"):\n\
         \x20                                       When Err (e):\n\
         \x20                                           Return e.\n\
         \x20                                       When Ok (tf):\n\
         \x20                                           Inspect jsonString(tf):\n\
         \x20                                               When Err (e):\n\
         \x20                                                   Return e.\n\
         \x20                                               When Ok (ts):\n\
         \x20                                                   Inspect jsonField(v, \"version\"):\n\
         \x20                                                       When Err (e):\n\
         \x20                                                           Return e.\n\
         \x20                                                       When Ok (vf):\n\
         \x20                                                           Inspect jsonInt(vf):\n\
         \x20                                                               When Err (e):\n\
         \x20                                                                   Return e.\n\
         \x20                                                               When Ok (vn):\n\
         \x20                                                                   If ks is equal to \"add\":\n\
         \x20                                                                       Return \"ADD text={{ts}} version={{vn}}\".\n\
         \x20                                                                   If ks is equal to \"remove\":\n\
         \x20                                                                       Return \"REMOVE text={{ts}} version={{vn}}\".\n\
         \x20                                                                   Return \"bad kind tag {{ks}}\".\n\
         \x20   Return \"unreachable\".\n\
         \n\
         ## Main\n\
         Show decodeCommand({}).\n",
        lg_text(json)
    )
}

#[test]
fn command_envelope_valid_decode() {
    assert_all_tiers(
        &decode_command_program(r#"{"kind":"add","text":"milk","version":3}"#),
        "ADD text=milk version=3",
    );
    assert_all_tiers(
        &decode_command_program(r#"{"kind":"remove","text":"eggs","version":9}"#),
        "REMOVE text=eggs version=9",
    );
}

#[test]
fn command_envelope_missing_field_rejects() {
    // No "version" field.
    assert_all_tiers(
        &decode_command_program(r#"{"kind":"add","text":"milk"}"#),
        "missing field \"version\"",
    );
}

#[test]
fn command_envelope_wrong_typed_field_rejects() {
    // "version" is a string, not an int.
    assert_all_tiers(
        &decode_command_program(r#"{"kind":"add","text":"milk","version":"3"}"#),
        "expected a JSON integer",
    );
    // "text" is a number, not a string.
    assert_all_tiers(
        &decode_command_program(r#"{"kind":"add","text":5,"version":3}"#),
        "expected a JSON string",
    );
}

#[test]
fn command_envelope_extra_key_rejects() {
    assert_all_tiers(
        &decode_command_program(r#"{"kind":"add","text":"milk","version":3,"admin":true}"#),
        "unexpected key \"admin\"",
    );
}

#[test]
fn command_envelope_bad_enum_tag_rejects() {
    assert_all_tiers(
        &decode_command_program(r#"{"kind":"drop","text":"milk","version":3}"#),
        "bad kind tag drop",
    );
}

#[test]
fn command_envelope_duplicate_key_rejects_before_any_field_read() {
    // A duplicate "version" key never reaches the typed read: strict parse rejects
    // the whole document first, so no decision is made on the last-write-wins value.
    let out = run_all_tiers(&decode_command_program(
        r#"{"kind":"add","text":"milk","version":3,"version":99}"#,
    ));
    assert!(out.error.is_none());
    assert!(
        out.output.trim().starts_with("parse: duplicate object key"),
        "duplicate key must be rejected at parse time, got: {}",
        out.output.trim()
    );
}

/// BOUNDED strict-path coverage (genuinely new — the conformance lock never runs
/// the STRICT parser at depth). Strictness scales: a valid 30-level document
/// strict-parses to a fixed point, and a duplicate key buried 30 levels deep is
/// still caught. The recursion runs on a big stack (the deep-nesting idiom).
#[test]
fn strict_scales_with_depth_and_catches_deep_duplicates() {
    on_big_stack(|| {
        // A valid deeply-nested object strict-parses and serializes identically.
        let deep_object = format!("{}1{}", r#"{"a":"#.repeat(30), "}".repeat(30));
        strict_accepts(&deep_object, &deep_object);

        // A duplicate key at the DEEPEST level is still rejected.
        let deep_dup = format!(
            "{}{}{}",
            r#"{"a":"#.repeat(29),
            r#"{"z":1,"z":2}"#,
            "}".repeat(29)
        );
        strict_rejects(&deep_dup);
    });
}
