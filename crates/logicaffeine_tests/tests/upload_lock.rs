//! ★ LOCK — `Upload … to URL` sends a file the way a server can read it. ★
//!
//! `Upload` was riding the typed-fetch path as a plain `POST` with the content as the whole body:
//! no framing, no `content-type`, nothing to tell a server it had received a FILE. Which makes it
//! indistinguishable from `Post`, and the sentence a lie.
//!
//! An upload is `multipart/form-data`, and the details that matter are the ones that go wrong
//! quietly: the boundary must not occur inside the content (or the server truncates the file at
//! whatever byte happens to match), the framing is CRLF, and the terminating boundary carries its
//! trailing `--`.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

/// Run a page that uploads `content`, and return the request it sent.
fn uploaded(content: &str) -> (String, String, Vec<(String, String)>) {
    let src = format!(
        "## A R has:\n    an ok, which is Int.\n\n\
         ## Component App\nState:\n    Let s be \"\".\n\
         Actions:\n    When the page loads:\n        \
         Upload \"{content}\" to \"/api/up\" as R, then done.\n    \
         To done (r: Result of R and HttpError):\n        Set s to \"back\".\n\
         View:\n    <p id=\"s\">{{s}}</p>\n\n\
         ## Main\nMount App.\n"
    );
    dom::reset();
    let out = tw_outcome(&src);
    assert_eq!(out.error, None, "the upload page must run:\n{src}");
    let reqs = dom::fetch_requests();
    assert_eq!(reqs.len(), 1, "exactly one request: {reqs:?}");
    (
        reqs[0].method.clone(),
        reqs[0].body.clone(),
        reqs[0].headers.clone(),
    )
}

fn content_type(headers: &[(String, String)]) -> String {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.clone())
        .unwrap_or_default()
}

#[test]
fn an_upload_is_a_post() {
    let (method, _, _) = uploaded("hello");
    assert_eq!(method, "POST", "an upload sends a body, so it is a POST");
}

#[test]
fn an_upload_announces_itself_as_multipart() {
    // Without this a server has no way to know it received a file rather than a JSON body — and
    // `Upload` becomes an alias for `Post`, which is the bug this lock exists to prevent.
    let (_, _, headers) = uploaded("hello");
    let ct = content_type(&headers);
    assert!(
        ct.starts_with("multipart/form-data"),
        "an upload's content-type says multipart; got {ct:?}"
    );
    assert!(
        ct.contains("boundary="),
        "and it carries the boundary the body uses; got {ct:?}"
    );
}

#[test]
fn the_body_is_framed_with_the_boundary_the_header_declares() {
    // The header and the body must agree. They are produced in different places and a mismatch
    // makes every upload fail on a real server while looking correct in a test that only reads
    // one of them.
    let (_, body, headers) = uploaded("hello");
    let ct = content_type(&headers);
    let boundary = ct
        .split("boundary=")
        .nth(1)
        .expect("the header names a boundary")
        .to_string();
    assert!(
        body.starts_with(&format!("--{boundary}\r\n")),
        "the body opens with the declared boundary:\n{body}"
    );
    assert!(
        body.ends_with(&format!("--{boundary}--\r\n")),
        "and closes with the terminating one, `--` and all:\n{body}"
    );
}

#[test]
fn the_part_carries_a_name_and_the_content() {
    let (_, body, _) = uploaded("hello");
    assert!(
        body.contains("Content-Disposition: form-data; name=\"file\""),
        "the part names itself so a server can find it:\n{body}"
    );
    assert!(body.contains("hello"), "and carries the content:\n{body}");
}

#[test]
fn the_framing_is_crlf_and_not_bare_newlines() {
    // A server parsing per the spec splits on CRLF. Bare newlines produce a body that looks right
    // in a terminal and parses as one enormous field.
    let (_, body, _) = uploaded("hello");
    assert!(
        body.contains("\r\n"),
        "multipart framing is CRLF:\n{body:?}"
    );
    assert!(
        !body.replace("\r\n", "").contains('\n'),
        "every newline in the framing must be a CRLF:\n{body:?}"
    );
}

#[test]
fn a_content_that_contains_the_boundary_gets_a_different_boundary() {
    // ★ THE ONE THAT CORRUPTS FILES SILENTLY. If the content happens to contain the boundary, a
    // server truncates the upload at that byte — and the failure looks like a corrupt file rather
    // than a framing bug, on some uploads and not others.
    let (_, plain, headers_plain) = uploaded("hello");
    let boundary = content_type(&headers_plain)
        .split("boundary=")
        .nth(1)
        .expect("a boundary")
        .to_string();
    let _ = plain;

    // Now upload content that contains that very boundary.
    let (_, body, headers) = uploaded(&boundary);
    let chosen = content_type(&headers)
        .split("boundary=")
        .nth(1)
        .expect("a boundary")
        .to_string();
    assert_ne!(
        chosen, boundary,
        "the content contains the default boundary, so a different one must be chosen"
    );
    // And whatever it chose, the content must not contain it.
    let payload = body
        .split("\r\n\r\n")
        .nth(1)
        .and_then(|p| p.split(&format!("\r\n--{chosen}")).next())
        .unwrap_or_default();
    assert!(
        !payload.contains(&chosen),
        "the chosen boundary must not occur in the content it delimits:\n{body}"
    );
}

#[test]
fn the_same_content_uploads_identically_every_time() {
    // The boundary must be DERIVED, not random: a random one makes every op-log lock a coin flip
    // and every recorded request unreproducible.
    let (_, a, ha) = uploaded("hello");
    let (_, b, hb) = uploaded("hello");
    assert_eq!(a, b, "the same upload must produce the same body");
    assert_eq!(
        content_type(&ha),
        content_type(&hb),
        "and the same content-type"
    );
}

/// ★ BOTH TIERS SEND THE SAME UPLOAD, byte for byte.
///
/// This used to be `#[ignore]`d beside a test asserting that the bytecode tier REFUSED a typed
/// request — its answer arrives in a closure and that tier could not call one it found at run
/// time. That was true of the DRIVING VERBS and not of this: an `on-click={action}` already
/// lowers to a closure the host calls, and a fetch completion is the same shape. So a typed
/// request now runs there, and the refusal it was measured against is gone.
///
/// ★ THE FRAMING IS WHERE THE TIERS COULD DRIFT, and it is what this asserts. An upload is
/// `multipart/form-data` — a boundary, a named part, CRLF framing — and the boundary is DERIVED
/// from the content so it cannot occur inside it. Two implementations of that derivation would
/// agree until the day a file contained the boundary, which is exactly the case that corrupts a
/// file silently. Both tiers now go through the same `multipartBody`/`multipartContentType` in
/// `web.lg`, and this holds them to it.
#[test]
fn both_tiers_send_the_same_upload() {
    let src = "## A R has:\n    an ok, which is Int.\n\n\
        ## Component App\nState:\n    Let s be \"\".\n\
        Actions:\n    When the page loads:\n        \
        Upload \"payload\" to \"/api/up\" as R, then done.\n    \
        To done (r: Result of R and HttpError):\n        Set s to \"back\".\n\
        View:\n    <p id=\"s\">{s}</p>\n\n## Main\nMount App.\n";
    dom::reset();
    assert_eq!(tw_outcome(src).error, None);
    let tw = dom::fetch_requests();
    dom::reset();
    assert_eq!(vm_outcome(src).error, None);
    let vm = dom::fetch_requests();
    assert_eq!(tw.len(), vm.len(), "both tiers send one request");
    assert_eq!(tw[0].body, vm[0].body, "byte-identical bodies");
    assert_eq!(tw[0].headers, vm[0].headers, "and identical headers");
}
