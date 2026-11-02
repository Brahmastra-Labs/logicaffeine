use super::*;

fn h(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

// ---- parse: happy paths -------------------------------------------------

#[test]
fn parse_simple_get() {
    let raw = b"GET /hello HTTP/1.1\r\nHost: example.com\r\n\r\n";
    let req = parse_request(raw).expect("valid GET");
    assert_eq!(req.method, "GET");
    assert_eq!(req.path, "/hello");
    assert!(req.query.is_empty());
    assert_eq!(
        req.headers.get("host").map(String::as_str),
        Some("example.com")
    );
    assert!(req.body.is_empty());
}

#[test]
fn parse_get_with_query_percent_decoded() {
    let raw = b"GET /search?q=hello%20world&n=2 HTTP/1.1\r\n\r\n";
    let req = parse_request(raw).expect("valid GET with query");
    assert_eq!(req.path, "/search");
    assert_eq!(req.query.get("q").map(String::as_str), Some("hello world"));
    assert_eq!(req.query.get("n").map(String::as_str), Some("2"));
}

#[test]
fn parse_query_plus_is_space() {
    let raw = b"GET /s?q=a+b HTTP/1.1\r\n\r\n";
    let req = parse_request(raw).expect("valid");
    assert_eq!(req.query.get("q").map(String::as_str), Some("a b"));
}

#[test]
fn parse_post_with_body_extracted_exactly() {
    let raw = b"POST /submit HTTP/1.1\r\nContent-Length: 5\r\n\r\nhello";
    let req = parse_request(raw).expect("valid POST");
    assert_eq!(req.method, "POST");
    assert_eq!(req.path, "/submit");
    assert_eq!(req.body, b"hello");
}

#[test]
fn parse_body_stops_at_content_length() {
    // Trailing bytes past Content-Length (e.g. a pipelined request) are not
    // part of this body.
    let raw = b"POST /x HTTP/1.1\r\nContent-Length: 3\r\n\r\nabcEXTRA";
    let req = parse_request(raw).expect("valid");
    assert_eq!(req.body, b"abc");
}

#[test]
fn parse_header_casing_normalized_to_lowercase() {
    let raw = b"GET / HTTP/1.1\r\nCoNtEnT-TyPe: text/html\r\n\r\n";
    let req = parse_request(raw).expect("valid");
    assert_eq!(
        req.headers.get("content-type").map(String::as_str),
        Some("text/html")
    );
}

#[test]
fn parse_header_value_with_colon_splits_on_first_only() {
    let raw = b"GET / HTTP/1.1\r\nReferer: http://x/y\r\n\r\n";
    let req = parse_request(raw).expect("valid");
    assert_eq!(
        req.headers.get("referer").map(String::as_str),
        Some("http://x/y")
    );
}

#[test]
fn parse_trims_optional_whitespace_around_value() {
    let raw = b"GET / HTTP/1.1\r\nX-Trim:   spaced   \r\n\r\n";
    let req = parse_request(raw).expect("valid");
    assert_eq!(
        req.headers.get("x-trim").map(String::as_str),
        Some("spaced")
    );
}

#[test]
fn parse_path_percent_decoded() {
    let raw = b"GET /a%2Fb%2Dc HTTP/1.1\r\n\r\n";
    let req = parse_request(raw).expect("valid");
    assert_eq!(req.path, "/a/b-c");
}

// ---- parse: error / boundary paths -------------------------------------

#[test]
fn parse_incomplete_when_no_header_terminator() {
    let raw = b"GET / HTTP/1.1\r\nHost: x\r\n"; // no blank line yet
    assert_eq!(parse_request(raw), Err(ParseError::Incomplete));
}

#[test]
fn parse_incomplete_when_body_short() {
    let raw = b"POST /x HTTP/1.1\r\nContent-Length: 10\r\n\r\nabc"; // only 3 of 10
    assert_eq!(parse_request(raw), Err(ParseError::Incomplete));
}

#[test]
fn parse_rejects_chunked_transfer_encoding() {
    let raw = b"POST /x HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n";
    assert_eq!(
        parse_request(raw),
        Err(ParseError::UnsupportedTransferEncoding)
    );
}

#[test]
fn parse_rejects_body_over_max() {
    let raw = format!(
        "POST /x HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
        MAX_BODY + 1
    );
    assert_eq!(parse_request(raw.as_bytes()), Err(ParseError::BodyTooLarge));
}

#[test]
fn parse_bad_request_line_missing_parts() {
    let raw = b"GARBAGE\r\n\r\n";
    assert!(matches!(parse_request(raw), Err(ParseError::BadRequest(_))));
}

#[test]
fn parse_bad_version() {
    let raw = b"GET / SPDY/9\r\n\r\n";
    assert!(matches!(parse_request(raw), Err(ParseError::BadRequest(_))));
}

#[test]
fn parse_headers_too_large_without_terminator() {
    // A huge blob with no blank line must not be buffered forever.
    let mut raw = b"GET / HTTP/1.1\r\n".to_vec();
    raw.extend(std::iter::repeat(b'x').take(MAX_HEADERS + 16));
    assert_eq!(parse_request(&raw), Err(ParseError::HeadersTooLarge));
}

// ---- duplicate headers: RFC 9110 comma-join, Content-Length agreement ---

#[test]
fn parse_request_duplicate_headers_comma_joined() {
    // RFC 9110 §5.2: repeated field lines combine into one comma-separated
    // value, IN ARRIVAL ORDER. Locked: "1, 2" — never first-wins, never
    // last-wins, never two map entries.
    let raw = b"GET / HTTP/1.1\r\nX-A: 1\r\nX-A: 2\r\n\r\n";
    let req = parse_request(raw).expect("valid");
    assert_eq!(req.headers.get("x-a").map(String::as_str), Some("1, 2"));
}

#[test]
fn parse_request_duplicate_headers_join_across_casings() {
    // The join keys on the case-insensitive name: X-A and x-a are the SAME
    // field, so their values join rather than shadow one another.
    let raw = b"GET / HTTP/1.1\r\nX-A: first\r\nx-a: second\r\nX-a: third\r\n\r\n";
    let req = parse_request(raw).expect("valid");
    assert_eq!(
        req.headers.get("x-a").map(String::as_str),
        Some("first, second, third")
    );
}

#[test]
fn parse_request_duplicate_identical_content_length_accepted() {
    // RFC 9110 §8.6: identical repeated values may collapse to one — and
    // MUST collapse (a joined "2, 2" would not parse as a length).
    let raw = b"POST /x HTTP/1.1\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\nok";
    let req = parse_request(raw).expect("valid");
    assert_eq!(req.body, b"ok");
}

#[test]
fn parse_request_rejects_conflicting_content_lengths() {
    // Differing repeated Content-Length is the classic request-smuggling
    // vector — a hard reject, never a quiet first- or last-wins.
    let raw = b"POST /x HTTP/1.1\r\nContent-Length: 3\r\nContent-Length: 5\r\n\r\nabcde";
    assert!(matches!(parse_request(raw), Err(ParseError::BadRequest(_))));
}

#[test]
fn parse_response_duplicate_headers_comma_joined() {
    // The client half mirrors the server half: same comma-join law.
    let raw = b"HTTP/1.1 200 OK\r\nX-A: 1\r\nX-A: 2\r\nContent-Length: 0\r\n\r\n";
    let resp = parse_response(raw).expect("valid");
    assert_eq!(resp.headers.get("x-a").map(String::as_str), Some("1, 2"));
}

// ---- request-side strictness: stray CR, oversized single header ---------

#[test]
fn parse_request_rejects_stray_cr_in_request_line() {
    // The head splits on CRLF, so a lone CR would otherwise smuggle bytes
    // into the version token unseen ("HTTP/1.1\rjunk" starts with "HTTP/").
    let raw = b"GET / HTTP/1.1\rjunk\r\n\r\n";
    assert!(matches!(parse_request(raw), Err(ParseError::BadRequest(_))));
}

#[test]
fn parse_request_rejects_stray_cr_in_header_line() {
    // A lone CR inside a header value must not reach the handler's map —
    // the request-side mirror of parse_response's stray-CR/LF rejection.
    let raw = b"GET / HTTP/1.1\r\nX-A: a\rb\r\n\r\n";
    assert!(matches!(parse_request(raw), Err(ParseError::BadRequest(_))));
}

#[test]
fn parse_request_single_oversized_header_rejected_with_terminator() {
    // One giant header line (terminator PRESENT) still trips MAX_HEADERS —
    // the with-terminator twin of the no-terminator buffering guard.
    let mut raw = b"GET / HTTP/1.1\r\nX-Big: ".to_vec();
    raw.extend(std::iter::repeat(b'x').take(MAX_HEADERS + 16));
    raw.extend_from_slice(b"\r\n\r\n");
    assert_eq!(parse_request(&raw), Err(ParseError::HeadersTooLarge));
}

// ---- serialize: byte-exact ---------------------------------------------

#[test]
fn serialize_200_byte_exact() {
    let resp = HttpResponse {
        status: 200,
        headers: h(&[("Content-Type", "text/plain")]),
        body: b"hi".to_vec(),
    };
    let out = serialize_response(&resp);
    assert_eq!(
        out,
        b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\nhi"
    );
}

#[test]
fn serialize_404_byte_exact() {
    let resp = HttpResponse {
        status: 404,
        headers: BTreeMap::new(),
        body: b"nope".to_vec(),
    };
    let out = serialize_response(&resp);
    assert_eq!(
        out,
        b"HTTP/1.1 404 Not Found\r\nContent-Length: 4\r\nConnection: close\r\n\r\nnope"
    );
}

#[test]
fn serialize_302_with_location_byte_exact() {
    let resp = HttpResponse {
        status: 302,
        headers: h(&[("Location", "/home")]),
        body: Vec::new(),
    };
    let out = serialize_response(&resp);
    assert_eq!(
        out,
        b"HTTP/1.1 302 Found\r\nLocation: /home\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
}

#[test]
fn serialize_ignores_user_supplied_framing_headers() {
    // A handler that sets Content-Length/Connection/Transfer-Encoding must
    // not produce duplicates or a wrong length — the writer owns these.
    let resp = HttpResponse {
        status: 200,
        headers: h(&[
            ("content-length", "999"),
            ("Connection", "keep-alive"),
            ("Transfer-Encoding", "chunked"),
        ]),
        body: b"abcd".to_vec(),
    };
    let out = serialize_response(&resp);
    assert_eq!(
        out,
        b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nabcd"
    );
}

#[test]
fn serialize_headers_are_deterministically_ordered() {
    // BTreeMap ordering makes the output a pure function of the map contents.
    let resp = HttpResponse {
        status: 200,
        headers: h(&[("X-Beta", "2"), ("X-Alpha", "1")]),
        body: Vec::new(),
    };
    let out = serialize_response(&resp);
    assert_eq!(
        out,
        b"HTTP/1.1 200 OK\r\nX-Alpha: 1\r\nX-Beta: 2\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
}

#[test]
fn serialize_binary_body_preserved() {
    let resp = HttpResponse {
        status: 200,
        headers: h(&[("Content-Type", "application/wasm")]),
        body: vec![0, 97, 115, 109],
    };
    let out = serialize_response(&resp);
    assert!(out.ends_with(&[0, 97, 115, 109]));
    assert!(out.windows(18).any(|w| w == b"Content-Length: 4\r"));
}

// ---- keep-alive law (shared by both execution tiers) --------------------

#[test]
fn serialize_response_conn_keep_alive_byte_exact() {
    let resp = HttpResponse {
        status: 200,
        headers: h(&[("Content-Type", "text/plain")]),
        body: b"hi".to_vec(),
    };
    assert_eq!(
        serialize_response_conn(&resp, true),
        b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: keep-alive\r\n\r\nhi"
    );
}

#[test]
fn serialize_response_is_the_close_form_of_serialize_response_conn() {
    for resp in response_battery() {
        assert_eq!(
            serialize_response(&resp),
            serialize_response_conn(&resp, false)
        );
    }
}

#[test]
fn serialize_response_conn_still_owns_the_connection_header() {
    // A handler-supplied Connection header never reaches the wire in either
    // disposition — the writer owns framing.
    let resp = HttpResponse {
        status: 200,
        headers: h(&[("Connection", "close")]),
        body: b"x".to_vec(),
    };
    assert_eq!(
        serialize_response_conn(&resp, true),
        b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nConnection: keep-alive\r\n\r\nx"
    );
}

#[test]
fn request_wants_keep_alive_battery() {
    let req_with = |conn: Option<&str>| {
        let mut headers = h(&[("host", "x")]);
        if let Some(v) = conn {
            headers.insert("connection".to_string(), v.to_string());
        }
        HttpRequest {
            method: "GET".to_string(),
            path: "/".to_string(),
            query: BTreeMap::new(),
            headers,
            body: Vec::new(),
        }
    };
    // HTTP/1.1 default: absent Connection header = persistent.
    assert!(request_wants_keep_alive(&req_with(None)));
    assert!(request_wants_keep_alive(&req_with(Some("keep-alive"))));
    // `close` in any casing, alone or inside a token list, ends the connection.
    assert!(!request_wants_keep_alive(&req_with(Some("close"))));
    assert!(!request_wants_keep_alive(&req_with(Some("Close"))));
    assert!(!request_wants_keep_alive(&req_with(Some("CLOSE"))));
    assert!(!request_wants_keep_alive(&req_with(Some(
        "keep-alive, close"
    ))));
    assert!(!request_wants_keep_alive(&req_with(Some(" close "))));
    // Other tokens alone do not close.
    assert!(request_wants_keep_alive(&req_with(Some("upgrade"))));
}

#[test]
fn parse_request_consuming_reports_the_exact_frame_length() {
    let raw = b"POST /x HTTP/1.1\r\nContent-Length: 3\r\n\r\nabcEXTRA";
    let (req, used) = parse_request_consuming(raw).expect("valid");
    assert_eq!(req.body, b"abc");
    assert_eq!(used, raw.len() - b"EXTRA".len());
}

#[test]
fn parse_request_consuming_frames_pipelined_requests() {
    // Two complete requests in one buffer: consuming the first's frame
    // leaves exactly the second — the persistent-connection drain law.
    let first = b"POST /a HTTP/1.1\r\nContent-Length: 2\r\n\r\nhi".to_vec();
    let second = b"GET /b HTTP/1.1\r\nHost: x\r\n\r\n".to_vec();
    let mut buf = first.clone();
    buf.extend_from_slice(&second);

    let (req1, used1) = parse_request_consuming(&buf).expect("first");
    assert_eq!(req1.path, "/a");
    assert_eq!(req1.body, b"hi");
    assert_eq!(used1, first.len());

    let (req2, used2) = parse_request_consuming(&buf[used1..]).expect("second");
    assert_eq!(req2.path, "/b");
    assert_eq!(used2, second.len());
    assert_eq!(used1 + used2, buf.len());
}

// ---- response-header law (shared by both execution tiers) ---------------

#[test]
fn response_header_allowed_accepts_ordinary_headers() {
    assert!(response_header_allowed("X-Request-Id", "abc-123"));
    assert!(response_header_allowed("cache-control", "no-store"));
    assert!(response_header_allowed("Location", "/home?a=b"));
    assert!(response_header_allowed("X-Empty-Value", ""));
}

#[test]
fn response_header_allowed_denies_hop_by_hop_any_casing() {
    for name in [
        "connection",
        "Connection",
        "CONNECTION",
        "content-length",
        "Content-Length",
        "transfer-encoding",
        "Transfer-Encoding",
        "keep-alive",
        "Keep-Alive",
        "upgrade",
        "Upgrade",
        "content-type",
        "Content-Type",
    ] {
        assert!(!response_header_allowed(name, "v"), "{name} must be denied");
    }
}

#[test]
fn response_header_allowed_denies_splitting_bytes_and_empty_name() {
    assert!(!response_header_allowed("", "v"));
    assert!(!response_header_allowed("X-A\r\nX-B", "v"));
    assert!(!response_header_allowed("X-A\n", "v"));
    assert!(!response_header_allowed("X-A", "v\r\nSet-Cookie: pwned"));
    assert!(!response_header_allowed("X-A", "v\ninjected"));
    assert!(!response_header_allowed("X-A", "v\0"));
    assert!(!response_header_allowed("X-A\0", "v"));
}

#[test]
fn safe_content_type_passes_clean_and_defaults_dirty() {
    assert_eq!(safe_content_type("application/json"), "application/json");
    assert_eq!(
        safe_content_type("text/html; charset=utf-8"),
        "text/html; charset=utf-8"
    );
    assert_eq!(safe_content_type(""), "text/plain");
    assert_eq!(safe_content_type("text/html\r\nX-Evil: 1"), "text/plain");
    assert_eq!(safe_content_type("a\nb"), "text/plain");
    assert_eq!(safe_content_type("a\0b"), "text/plain");
}

// ---- round trip ---------------------------------------------------------

#[test]
fn roundtrip_request_headers_survive() {
    let raw = b"POST /api?x=1 HTTP/1.1\r\nHost: h\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}";
    let req = parse_request(raw).expect("valid");
    assert_eq!(req.method, "POST");
    assert_eq!(req.path, "/api");
    assert_eq!(req.query.get("x").map(String::as_str), Some("1"));
    assert_eq!(
        req.headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(req.body, b"{}");
}

// ---- client half: batteries + canonical forms ---------------------------

/// A request battery covering the round-trip battlefield: empty body, binary
/// body with 0x00/0xFF (and an embedded `\r\n\r\n`, which must never confuse
/// framing), query strings needing percent-encoding, and weird-but-legal
/// header values (colons, quotes, internal tabs).
fn request_battery() -> Vec<HttpRequest> {
    vec![
        HttpRequest {
            method: "GET".to_string(),
            path: "/".to_string(),
            query: BTreeMap::new(),
            headers: h(&[("host", "example.com")]),
            body: Vec::new(),
        },
        HttpRequest {
            method: "POST".to_string(),
            path: "/api/v1/items".to_string(),
            query: h(&[("q", "hello world"), ("lang", "en-US")]),
            headers: h(&[("host", "h"), ("content-type", "application/json")]),
            body: b"{\"k\":1}".to_vec(),
        },
        HttpRequest {
            method: "PUT".to_string(),
            path: "/upload".to_string(),
            query: BTreeMap::new(),
            headers: h(&[("host", "h")]),
            body: vec![
                0x00, 0xFF, 0x13, 0x37, b'\r', b'\n', b'\r', b'\n', 0x00, 0xFF,
            ],
        },
        HttpRequest {
            method: "GET".to_string(),
            path: "/söme path/with?mark".to_string(),
            query: h(&[("a&b", "c=d"), ("plus+", "sp ace"), ("uni", "héllo")]),
            headers: h(&[("host", "h")]),
            body: Vec::new(),
        },
        HttpRequest {
            method: "DELETE".to_string(),
            path: "/x".to_string(),
            query: BTreeMap::new(),
            headers: h(&[
                ("host", "h:8080"),
                ("referer", "http://h:8080/p?a=b"),
                ("x-quoted", "a=\"b, c\"; d"),
                ("x-tab", "a\tb"),
            ]),
            body: Vec::new(),
        },
    ]
}

/// A response battery: 200/204/404/500 plus an unknown 599 (empty reason
/// phrase on the wire), empty body, and a binary body with 0x00/0xFF and an
/// embedded `\r\n\r\n`.
fn response_battery() -> Vec<HttpResponse> {
    vec![
        HttpResponse {
            status: 200,
            headers: h(&[("content-type", "text/plain")]),
            body: b"hello".to_vec(),
        },
        HttpResponse {
            status: 204,
            headers: BTreeMap::new(),
            body: Vec::new(),
        },
        HttpResponse {
            status: 404,
            headers: h(&[("content-type", "application/json")]),
            body: b"{\"err\":\"nope\"}".to_vec(),
        },
        HttpResponse {
            status: 500,
            headers: BTreeMap::new(),
            body: vec![0x00, 0xFF, 0x7F, 0x80, b'\r', b'\n', b'\r', b'\n', 0x00],
        },
        HttpResponse {
            status: 599,
            headers: h(&[("x-weird", "a=\"b, c\"; d: e")]),
            body: b"?".to_vec(),
        },
    ]
}

/// What a message looks like after one trip through its writer + parser: the
/// parser lowercases header names, and the writer owns the framing headers
/// (it drops caller-supplied ones and emits an accurate `Content-Length` plus
/// `Connection: close`, which the parser then surfaces in the map).
fn canonical_headers(
    headers: &BTreeMap<String, String>,
    body_len: usize,
) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = headers
        .iter()
        .filter(|(k, _)| {
            !matches!(
                k.to_ascii_lowercase().as_str(),
                "content-length" | "connection" | "transfer-encoding"
            )
        })
        .map(|(k, v)| (k.to_ascii_lowercase(), v.clone()))
        .collect();
    out.insert("content-length".to_string(), body_len.to_string());
    out.insert("connection".to_string(), "close".to_string());
    out
}

fn canonical_request(req: &HttpRequest) -> HttpRequest {
    HttpRequest {
        method: req.method.clone(),
        path: req.path.clone(),
        query: req.query.clone(),
        headers: canonical_headers(&req.headers, req.body.len()),
        body: req.body.clone(),
    }
}

fn canonical_response(resp: &HttpResponse) -> HttpResponse {
    HttpResponse {
        status: resp.status,
        headers: canonical_headers(&resp.headers, resp.body.len()),
        body: resp.body.clone(),
    }
}

// ---- client half: serialize_request byte-exact --------------------------

#[test]
fn serialize_request_get_byte_exact() {
    let req = HttpRequest {
        method: "GET".to_string(),
        path: "/hello".to_string(),
        query: BTreeMap::new(),
        headers: h(&[("host", "example.com")]),
        body: Vec::new(),
    };
    assert_eq!(
        serialize_request(&req),
        b"GET /hello HTTP/1.1\r\nhost: example.com\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
    );
}

#[test]
fn serialize_request_post_with_query_and_body_byte_exact() {
    let req = HttpRequest {
        method: "POST".to_string(),
        path: "/api/search".to_string(),
        query: h(&[("q", "hello world"), ("n", "2")]),
        headers: h(&[("content-type", "application/json"), ("host", "h")]),
        body: b"{}".to_vec(),
    };
    assert_eq!(
        serialize_request(&req),
        b"POST /api/search?n=2&q=hello%20world HTTP/1.1\r\ncontent-type: application/json\r\nhost: h\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".to_vec()
    );
}

#[test]
fn serialize_request_percent_encodes_target() {
    let req = HttpRequest {
        method: "GET".to_string(),
        path: "/a b/c?d".to_string(),
        query: h(&[("a&b", "c=d")]),
        headers: BTreeMap::new(),
        body: Vec::new(),
    };
    let out = serialize_request(&req);
    assert!(out.starts_with(b"GET /a%20b/c%3Fd?a%26b=c%3Dd HTTP/1.1\r\n"));
}

#[test]
fn serialize_request_ignores_user_supplied_framing_headers() {
    // A caller that sets Content-Length/Connection/Transfer-Encoding must
    // not produce duplicates or a wrong length — the writer owns these,
    // exactly as serialize_response does.
    let req = HttpRequest {
        method: "POST".to_string(),
        path: "/x".to_string(),
        query: BTreeMap::new(),
        headers: h(&[
            ("content-length", "999"),
            ("Connection", "keep-alive"),
            ("Transfer-Encoding", "chunked"),
        ]),
        body: b"abcd".to_vec(),
    };
    assert_eq!(
        serialize_request(&req),
        b"POST /x HTTP/1.1\r\nContent-Length: 4\r\nConnection: close\r\n\r\nabcd".to_vec()
    );
}

// ---- client half: parse_response happy paths ----------------------------

#[test]
fn parse_response_simple_200() {
    let raw = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\nhi";
    let resp = parse_response(raw).expect("valid 200");
    assert_eq!(resp.status, 200);
    assert_eq!(
        resp.headers.get("content-type").map(String::as_str),
        Some("text/plain")
    );
    assert_eq!(resp.body, b"hi");
}

#[test]
fn parse_response_reason_phrase_with_spaces() {
    let raw = b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n";
    let resp = parse_response(raw).expect("valid 500");
    assert_eq!(resp.status, 500);
    assert!(resp.body.is_empty());
}

#[test]
fn parse_response_empty_reason_with_trailing_space() {
    // serialize_response emits `HTTP/1.1 599 \r\n` for codes it has no
    // reason phrase for — the parser must take its own writer's output.
    let raw = b"HTTP/1.1 599 \r\nContent-Length: 0\r\n\r\n";
    assert_eq!(parse_response(raw).expect("valid 599").status, 599);
}

#[test]
fn parse_response_tolerates_missing_reason() {
    // RFC 7230 requires the SP after the code, but robustness (and httparse)
    // accept its absence; the reason carries no information either way.
    let raw = b"HTTP/1.1 204\r\n\r\n";
    assert_eq!(parse_response(raw).expect("valid 204").status, 204);
}

#[test]
fn parse_response_missing_content_length_is_empty_body() {
    // Symmetry decision: serialize_response ALWAYS emits Content-Length, so
    // the parser never falls back to read-to-end-of-input — a missing
    // Content-Length means an empty body, mirroring parse_request's default.
    let raw = b"HTTP/1.1 200 OK\r\n\r\ntrailing-not-body";
    let resp = parse_response(raw).expect("valid");
    assert!(resp.body.is_empty());
}

#[test]
fn parse_response_body_stops_at_content_length() {
    let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabcEXTRA";
    assert_eq!(parse_response(raw).expect("valid").body, b"abc");
}

#[test]
fn parse_response_header_casing_normalized_to_lowercase() {
    let raw = b"HTTP/1.1 200 OK\r\nCoNtEnT-TyPe: text/html\r\nContent-Length: 0\r\n\r\n";
    let resp = parse_response(raw).expect("valid");
    assert_eq!(
        resp.headers.get("content-type").map(String::as_str),
        Some("text/html")
    );
}

#[test]
fn parse_response_binary_body_preserved() {
    let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n\x00\xFF\x00\xFF";
    assert_eq!(
        parse_response(raw).expect("valid").body,
        vec![0x00, 0xFF, 0x00, 0xFF]
    );
}

#[test]
fn parse_response_duplicate_identical_content_length_accepted() {
    // RFC 9110 §8.6: identical repeated values may collapse to one; only
    // DIFFERING values are the smuggling hazard.
    let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\nok";
    assert_eq!(parse_response(raw).expect("valid").body, b"ok");
}

// ---- client half: parse_response error / boundary paths -----------------

#[test]
fn parse_response_incomplete_when_no_header_terminator() {
    let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n"; // no blank line yet
    assert_eq!(parse_response(raw), Err(ParseError::Incomplete));
}

#[test]
fn parse_response_incomplete_when_body_short() {
    let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabc"; // only 3 of 10
    assert_eq!(parse_response(raw), Err(ParseError::Incomplete));
}

#[test]
fn parse_response_rejects_chunked_transfer_encoding() {
    let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n";
    assert_eq!(
        parse_response(raw),
        Err(ParseError::UnsupportedTransferEncoding)
    );
}

#[test]
fn parse_response_rejects_body_over_max() {
    let raw = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
        MAX_BODY + 1
    );
    assert_eq!(
        parse_response(raw.as_bytes()),
        Err(ParseError::BodyTooLarge)
    );
}

#[test]
fn parse_response_headers_too_large_without_terminator() {
    let mut raw = b"HTTP/1.1 200 OK\r\n".to_vec();
    raw.extend(std::iter::repeat(b'x').take(MAX_HEADERS + 16));
    assert_eq!(parse_response(&raw), Err(ParseError::HeadersTooLarge));
}

#[test]
fn parse_response_rejects_conflicting_content_lengths() {
    let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nContent-Length: 5\r\n\r\nabcde";
    assert!(matches!(
        parse_response(raw),
        Err(ParseError::BadResponse(_))
    ));
}

#[test]
fn parse_response_rejects_status_outside_class_range() {
    assert!(matches!(
        parse_response(b"HTTP/1.1 099 Low\r\n\r\n"),
        Err(ParseError::BadResponse(_))
    ));
    assert!(matches!(
        parse_response(b"HTTP/1.1 600 High\r\n\r\n"),
        Err(ParseError::BadResponse(_))
    ));
}

/// Every entry the malformed-corpus + differential tests share: the raw
/// bytes, and — when our verdict legitimately differs from httparse's — the
/// exact `(ours, theirs, reason)` triple that is allowed. Anything else is
/// a real disagreement and fails the differential.
struct OracleCase {
    name: &'static str,
    wire: Vec<u8>,
    allowed_divergence: Option<(Verdict, Verdict, &'static str)>,
}

fn malformed_response_corpus() -> Vec<OracleCase> {
    vec![
        OracleCase {
            name: "pure garbage with a terminator",
            wire: b"\x00\x01\x02garbage\x1b\r\n\r\n".to_vec(),
            allowed_divergence: None,
        },
        OracleCase {
            name: "non-numeric status",
            wire: b"HTTP/1.1 abc OK\r\nContent-Length: 0\r\n\r\n".to_vec(),
            allowed_divergence: None,
        },
        OracleCase {
            name: "two-digit status",
            wire: b"HTTP/1.1 42 OK\r\n\r\n".to_vec(),
            allowed_divergence: None,
        },
        OracleCase {
            name: "four-digit status",
            wire: b"HTTP/1.1 9999 OK\r\n\r\n".to_vec(),
            allowed_divergence: None,
        },
        OracleCase {
            name: "three-digit status outside 100-599",
            wire: b"HTTP/1.1 999 Whatever\r\n\r\n".to_vec(),
            allowed_divergence: Some((
                Verdict::Reject,
                Verdict::Accept,
                "we enforce the RFC 9110 status classes (100-599); httparse accepts any 3DIGIT code",
            )),
        },
        OracleCase {
            name: "negative Content-Length",
            wire: b"HTTP/1.1 200 OK\r\nContent-Length: -1\r\n\r\n".to_vec(),
            allowed_divergence: Some((
                Verdict::Reject,
                Verdict::Accept,
                "httparse stops at the header block and never interprets Content-Length; body framing is ours alone",
            )),
        },
        OracleCase {
            name: "overflowing Content-Length",
            wire: b"HTTP/1.1 200 OK\r\nContent-Length: 99999999999999999999\r\n\r\n".to_vec(),
            allowed_divergence: Some((
                Verdict::Reject,
                Verdict::Accept,
                "httparse stops at the header block and never interprets Content-Length; body framing is ours alone",
            )),
        },
        OracleCase {
            name: "two conflicting Content-Lengths",
            wire: b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nContent-Length: 5\r\n\r\nabcde".to_vec(),
            allowed_divergence: Some((
                Verdict::Reject,
                Verdict::Accept,
                "httparse stops at the header block and never interprets Content-Length; body framing is ours alone",
            )),
        },
        OracleCase {
            name: "declared body over MAX_BODY",
            wire: format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", MAX_BODY + 1).into_bytes(),
            allowed_divergence: Some((
                Verdict::Reject,
                Verdict::Accept,
                "we cap declared bodies at MAX_BODY; httparse never interprets Content-Length",
            )),
        },
        OracleCase {
            name: "header without colon",
            wire: b"HTTP/1.1 200 OK\r\nNoColonHere\r\n\r\n".to_vec(),
            allowed_divergence: None,
        },
        OracleCase {
            name: "CR without LF",
            wire: b"HTTP/1.1 200 OK\rContent-Length: 0\r\n\r\n".to_vec(),
            allowed_divergence: None,
        },
        OracleCase {
            name: "double space before status code",
            wire: b"HTTP/1.1  200 OK\r\n\r\n".to_vec(),
            allowed_divergence: None,
        },
        OracleCase {
            name: "chunked transfer coding",
            wire: b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n".to_vec(),
            allowed_divergence: Some((
                Verdict::Reject,
                Verdict::Accept,
                "we reject non-identity transfer codings (v1 scope) rather than mishandle them; httparse only shapes the header block",
            )),
        },
        OracleCase {
            name: "non-UTF-8 header bytes",
            wire: b"HTTP/1.1 200 OK\r\nX-Bin: \xFF\xFE\r\n\r\n".to_vec(),
            allowed_divergence: Some((
                Verdict::Reject,
                Verdict::Accept,
                "we require a UTF-8 header block (values become Rust Strings); httparse permits raw obs-text bytes in values",
            )),
        },
        OracleCase {
            name: "empty head (terminator first)",
            wire: b"\r\n\r\n".to_vec(),
            allowed_divergence: Some((
                Verdict::Reject,
                Verdict::More,
                "httparse skips leading empty lines (RFC 7230 robustness) and waits for a status line; we require the status line first",
            )),
        },
    ]
}

#[test]
fn parse_response_malformed_corpus_rejected_never_incomplete() {
    for case in malformed_response_corpus() {
        match parse_response(&case.wire) {
            Err(ParseError::Incomplete) => panic!("{}: Incomplete, want a hard reject", case.name),
            Err(_) => {}
            Ok(resp) => panic!("{}: wrongly accepted as {resp:?}", case.name),
        }
    }
}

// ---- client half: round trips + torn-input sweeps -----------------------

#[test]
fn roundtrip_serialize_request_then_parse_equals_canonical_original() {
    for req in request_battery() {
        let wire = serialize_request(&req);
        let parsed = parse_request(&wire).unwrap_or_else(|e| panic!("battery {req:?}: {e:?}"));
        assert_eq!(parsed, canonical_request(&req));
    }
}

#[test]
fn roundtrip_serialize_response_then_parse_equals_canonical_original() {
    for resp in response_battery() {
        let wire = serialize_response(&resp);
        let parsed = parse_response(&wire).unwrap_or_else(|e| panic!("battery {resp:?}: {e:?}"));
        assert_eq!(parsed, canonical_response(&resp));
    }
}

#[test]
fn torn_response_every_strict_prefix_is_incomplete_never_a_wrong_success() {
    for resp in response_battery() {
        let wire = serialize_response(&resp);
        for cut in 0..wire.len() {
            assert_eq!(
                parse_response(&wire[..cut]),
                Err(ParseError::Incomplete),
                "prefix {cut}/{} of {resp:?}",
                wire.len()
            );
        }
        assert_eq!(parse_response(&wire), Ok(canonical_response(&resp)));
    }
}

#[test]
fn torn_request_every_strict_prefix_is_incomplete_never_a_wrong_success() {
    for req in request_battery() {
        let wire = serialize_request(&req);
        for cut in 0..wire.len() {
            assert_eq!(
                parse_request(&wire[..cut]),
                Err(ParseError::Incomplete),
                "prefix {cut}/{} of {req:?}",
                wire.len()
            );
        }
        assert_eq!(parse_request(&wire), Ok(canonical_request(&req)));
    }
}

// ---- client half: differential oracle vs httparse -----------------------

/// A parser's judgement of a byte buffer, comparable across implementations:
/// ours maps `Ok` → Accept, `Incomplete` → More, other errors → Reject;
/// httparse maps `Complete` → Accept, `Partial` → More, `Err` → Reject.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Accept,
    More,
    Reject,
}

fn our_response_verdict(bytes: &[u8]) -> Verdict {
    match parse_response(bytes) {
        Ok(_) => Verdict::Accept,
        Err(ParseError::Incomplete) => Verdict::More,
        Err(_) => Verdict::Reject,
    }
}

fn httparse_response_verdict(bytes: &[u8]) -> Verdict {
    let mut headers = [httparse::EMPTY_HEADER; 128];
    let mut resp = httparse::Response::new(&mut headers);
    match resp.parse(bytes) {
        Ok(httparse::Status::Complete(_)) => Verdict::Accept,
        Ok(httparse::Status::Partial) => Verdict::More,
        Err(_) => Verdict::Reject,
    }
}

#[test]
fn differential_httparse_response_verdicts_agree_or_allowlisted() {
    // Malformed corpus: verdicts must agree exactly, or diverge exactly as
    // the case declares — a declared divergence that stops manifesting
    // means the allowlist entry is stale and must be deleted (no silent
    // slack in either direction).
    for case in malformed_response_corpus() {
        let ours = our_response_verdict(&case.wire);
        let theirs = httparse_response_verdict(&case.wire);
        match case.allowed_divergence {
            None => assert_eq!(
                ours,
                theirs,
                "{}: verdicts disagree without an allowlist entry, input {:?}",
                case.name,
                String::from_utf8_lossy(&case.wire)
            ),
            Some((allowed_ours, allowed_theirs, reason)) => assert!(
                ours == allowed_ours && theirs == allowed_theirs,
                "{}: expected the documented divergence ({allowed_ours:?} vs {allowed_theirs:?} — {reason}) but got ours {ours:?}, httparse {theirs:?}",
                case.name
            ),
        }
    }
    // Torn corpus: every prefix of every battery response. The ONLY
    // legitimate divergence is (More, Accept): a complete head with a torn
    // Content-Length body is Complete to httparse (it never frames bodies)
    // but Incomplete to us. Anything else is a bug.
    for resp in response_battery() {
        let wire = serialize_response(&resp);
        for cut in 0..=wire.len() {
            let ours = our_response_verdict(&wire[..cut]);
            let theirs = httparse_response_verdict(&wire[..cut]);
            assert!(
                ours == theirs || (ours == Verdict::More && theirs == Verdict::Accept),
                "torn prefix {cut}/{} of {resp:?}: ours {ours:?}, httparse {theirs:?}",
                wire.len()
            );
        }
    }
}

// ---- fuzz-ish: deterministic no-panic sweeps ----------------------------

/// xorshift64 — a fixed-seed deterministic PRNG (no clocks, no OS entropy),
/// so the sweep is byte-for-byte reproducible.
struct XorShift64(u64);

impl XorShift64 {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

/// Phase 1: 10_000 random buffers up to 512 bytes. Phase 2 (same PRNG):
/// mutations + truncations of real serialized messages, so the deep parse
/// paths get exercised, not just the no-terminator path.
fn fuzz_sweep(seed: u64, wires: &[Vec<u8>], parse: impl Fn(&[u8])) {
    let mut rng = XorShift64(seed);
    for _ in 0..10_000 {
        let len = (rng.next() % 513) as usize;
        let buf: Vec<u8> = (0..len).map(|_| (rng.next() >> 32) as u8).collect();
        parse(&buf);
    }
    for wire in wires {
        for _ in 0..2_000 {
            let mut m = wire.clone();
            let flips = 1 + (rng.next() % 4) as usize;
            for _ in 0..flips {
                let i = (rng.next() as usize) % m.len();
                m[i] = (rng.next() >> 24) as u8;
            }
            let cut = (rng.next() as usize) % (m.len() + 1);
            parse(&m[..cut]);
            parse(&m);
        }
    }
}

#[test]
fn fuzz_parse_response_never_panics() {
    let wires: Vec<Vec<u8>> = response_battery().iter().map(serialize_response).collect();
    fuzz_sweep(0x9E37_79B9_7F4A_7C15, &wires, |b| {
        let _ = parse_response(b);
    });
}

#[test]
fn fuzz_parse_request_never_panics() {
    let wires: Vec<Vec<u8>> = request_battery().iter().map(serialize_request).collect();
    fuzz_sweep(0xD1B5_4A32_D192_ED03, &wires, |b| {
        let _ = parse_request(b);
    });
}
