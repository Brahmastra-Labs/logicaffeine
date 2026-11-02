//! HTTP/1.1 wire format — hand-rolled parsers and writers over plain byte
//! buffers. No hyper, no allocations beyond the owned result. The server half
//! is [`parse_request`] + [`serialize_response`]; the client half is its exact
//! mirror, [`serialize_request`] + [`parse_response`] — same error taxonomy,
//! same limits, same [`ParseError::Incomplete`] retry contract.
//!
//! Scope (v1, deliberately bounded — documented so callers know the edges):
//! - Methods: any token (GET/POST/… are not restricted).
//! - Bodies: framed by `Content-Length` only. `Transfer-Encoding: chunked` is
//!   **rejected** ([`ParseError::UnsupportedTransferEncoding`]) rather than
//!   silently mishandled.
//! - Header names are normalized to **lowercase** on parse so lookups are
//!   case-insensitive (HTTP header names are case-insensitive by RFC 7230).
//! - Connection reuse: the SERVER speaks HTTP/1.1 persistent connections —
//!   [`request_wants_keep_alive`] is the one law both execution tiers apply
//!   (keep-alive unless the client's `Connection` header lists `close`), and
//!   [`serialize_response_conn`] emits the matching `Connection:` header.
//!   [`serialize_response`] remains the one-shot (`Connection: close`) form,
//!   and the CLIENT half ([`serialize_request`]) still requests one-shot
//!   exchanges — a deliberate v1 client scope.
//! - Sizes are bounded: [`MAX_BODY`] caps bodies (requests and responses
//!   alike), [`MAX_HEADERS`] caps the header block, so a malicious peer cannot
//!   exhaust memory.
//! - Framing symmetry: both writers own the framing headers (an accurate
//!   `Content-Length` computed from the body, `Connection: close`, never
//!   `Transfer-Encoding`), so neither parser needs read-to-end-of-input
//!   framing — a missing `Content-Length` always means an empty body, and torn
//!   input is always [`ParseError::Incomplete`], never a guess at EOF.
//!
//! [`serialize_response`] always emits an accurate `Content-Length` (computed
//! from the body) and `Connection: close`, and omits `Date` / other volatile
//! headers, so a response is a **pure function of the handler's output** — which
//! is what lets the cross-tier differential assert byte-for-byte equality.
//! [`serialize_request`] obeys the same law on the client side.

use std::collections::BTreeMap;

/// Largest request body accepted, in bytes. Requests declaring a larger
/// `Content-Length` are rejected with [`ParseError::BodyTooLarge`].
pub const MAX_BODY: usize = 8 * 1024 * 1024;

/// Largest header block (request line + headers, up to the blank line) accepted.
pub const MAX_HEADERS: usize = 64 * 1024;

/// A parsed HTTP request. `path` and `query` values are percent-decoded; header
/// keys are lowercased. `body` is the raw bytes (a LOGOS `Request.body: Text` is
/// derived from this at the interpreter/AOT bridge).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub query: BTreeMap<String, String>,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

/// A response to serialize. `status` is the numeric code; `headers` are written
/// verbatim (except `Content-Length`/`Connection`/`Transfer-Encoding`, which the
/// writer owns); `body` is raw bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

/// Why a byte buffer could not be parsed into a complete request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// Not enough bytes yet — the header terminator or the full body has not
    /// arrived. The server should read more and retry.
    Incomplete,
    /// The request line or a header was malformed.
    BadRequest(String),
    /// The status line or a header of a response was malformed — the
    /// client-side mirror of [`ParseError::BadRequest`].
    BadResponse(String),
    /// `Transfer-Encoding: chunked` (or any non-identity encoding) — not
    /// supported by the v1 hand-rolled core.
    UnsupportedTransferEncoding,
    /// Declared `Content-Length` exceeds [`MAX_BODY`].
    BodyTooLarge,
    /// The header block exceeded [`MAX_HEADERS`] before a terminator appeared.
    HeadersTooLarge,
}

/// Parse a complete HTTP/1.1 request from `bytes`.
///
/// Returns [`ParseError::Incomplete`] if `bytes` does not yet contain the full
/// header block (`\r\n\r\n`) plus a `Content-Length`-worth of body — the caller
/// reads more and calls again.
pub fn parse_request(bytes: &[u8]) -> Result<HttpRequest, ParseError> {
    parse_request_consuming(bytes).map(|(req, _)| req)
}

/// [`parse_request`] plus the number of bytes the request occupied — the
/// keep-alive keystone: a persistent connection drains exactly `consumed`
/// bytes and keeps the remainder (a pipelined next request) buffered.
pub fn parse_request_consuming(bytes: &[u8]) -> Result<(HttpRequest, usize), ParseError> {
    // Locate the blank line that ends the header block. Until it arrives we do
    // not have a parseable request — but a peer that never sends it must not be
    // buffered without bound.
    let header_end = match find_subslice(bytes, b"\r\n\r\n") {
        Some(i) => i,
        None => {
            return if bytes.len() > MAX_HEADERS {
                Err(ParseError::HeadersTooLarge)
            } else {
                Err(ParseError::Incomplete)
            };
        }
    };
    if header_end > MAX_HEADERS {
        return Err(ParseError::HeadersTooLarge);
    }
    let body_start = header_end + 4;

    let head = std::str::from_utf8(&bytes[..header_end])
        .map_err(|_| ParseError::BadRequest("non-UTF-8 header block".to_string()))?;
    let mut lines = head.split("\r\n");

    // Request line: exactly `METHOD SP request-target SP HTTP-version`. A
    // stray CR (the head splits on CRLF, so one can lurk) would smuggle bytes
    // into the version token unseen — reject it, mirroring parse_response.
    let request_line = lines.next().unwrap_or("");
    if request_line.contains('\r') || request_line.contains('\n') {
        return Err(ParseError::BadRequest(format!(
            "stray CR/LF in request line: {request_line:?}"
        )));
    }
    let mut parts = request_line.split(' ');
    let (method, target, version) = match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(m), Some(t), Some(v), None) if !m.is_empty() => (m, t, v),
        _ => {
            return Err(ParseError::BadRequest(format!(
                "malformed request line: {request_line:?}"
            )))
        }
    };
    if !version.starts_with("HTTP/") {
        return Err(ParseError::BadRequest(format!(
            "unsupported version: {version:?}"
        )));
    }

    // Split target into a percent-decoded path and a decoded query map.
    let (path_raw, query_raw) = match target.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (target, None),
    };
    let path = percent_decode(path_raw, false);
    let mut query = BTreeMap::new();
    if let Some(q) = query_raw {
        for pair in q.split('&').filter(|p| !p.is_empty()) {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            query.insert(percent_decode(k, true), percent_decode(v, true));
        }
    }

    // Headers: lowercase names (case-insensitive per RFC 7230), split on the
    // first colon only so values may contain colons (e.g. a URL). Repeated
    // field lines COMMA-JOIN in arrival order (RFC 9110 §5.2) — except
    // Content-Length, where repeats must agree (RFC 9110 §8.6; differing
    // values are the classic request-smuggling vector, a hard reject) and
    // collapse to one (a joined "2, 2" would not parse as a length). A stray
    // CR inside a line is rejected, mirroring parse_response.
    let mut headers: BTreeMap<String, String> = BTreeMap::new();
    for line in lines.filter(|l| !l.is_empty()) {
        if line.contains('\r') || line.contains('\n') {
            return Err(ParseError::BadRequest(format!(
                "stray CR/LF in header: {line:?}"
            )));
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| ParseError::BadRequest(format!("malformed header: {line:?}")))?;
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim().to_string();
        match headers.entry(name) {
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert(value);
            }
            std::collections::btree_map::Entry::Occupied(mut slot) => {
                if slot.key() == "content-length" {
                    if *slot.get() != value {
                        return Err(ParseError::BadRequest(format!(
                            "conflicting Content-Length: {:?} vs {value:?}",
                            slot.get()
                        )));
                    }
                } else {
                    let joined = format!("{}, {value}", slot.get());
                    slot.insert(joined);
                }
            }
        }
    }

    // Reject any non-identity transfer coding rather than mishandle it.
    if let Some(te) = headers.get("transfer-encoding") {
        if te
            .split(',')
            .any(|t| !t.trim().eq_ignore_ascii_case("identity"))
        {
            return Err(ParseError::UnsupportedTransferEncoding);
        }
    }

    // Body is framed by Content-Length (default 0), bounded by MAX_BODY.
    let content_length = match headers.get("content-length") {
        Some(cl) => cl
            .trim()
            .parse::<usize>()
            .map_err(|_| ParseError::BadRequest(format!("bad Content-Length: {cl:?}")))?,
        None => 0,
    };
    if content_length > MAX_BODY {
        return Err(ParseError::BodyTooLarge);
    }
    if bytes.len() < body_start + content_length {
        return Err(ParseError::Incomplete);
    }
    let body = bytes[body_start..body_start + content_length].to_vec();

    Ok((
        HttpRequest {
            method: method.to_string(),
            path,
            query,
            headers,
            body,
        },
        body_start + content_length,
    ))
}

/// Whether the server should keep the connection open after answering `req` —
/// THE ONE keep-alive law both execution tiers (the interpreter's serve loop
/// and the AOT `serve_on`) apply, so their `Connection:` responses can never
/// drift: HTTP/1.1 connections persist unless the client's `Connection` header
/// token list contains `close` (case-insensitive).
///
/// Scope note: [`parse_request`] does not retain the version token (it accepts
/// any `HTTP/*`), so HTTP/1.0's implicit-close default is not modeled — every
/// parsed request gets HTTP/1.1 persistence semantics.
pub fn request_wants_keep_alive(req: &HttpRequest) -> bool {
    match req.headers.get("connection") {
        Some(v) => !v.split(',').any(|t| t.trim().eq_ignore_ascii_case("close")),
        None => true,
    }
}

/// Serialize a response to bytes: status line, the response's headers (minus the
/// writer-owned ones), an accurate `Content-Length`, `Connection: close`, the
/// blank line, then the body — the one-shot form of [`serialize_response_conn`].
pub fn serialize_response(resp: &HttpResponse) -> Vec<u8> {
    serialize_response_conn(resp, false)
}

/// Serialize a response with an explicit connection disposition: the writer
/// owns the `Connection:` header (`keep-alive` to hold the connection open for
/// the next request, `close` to end it) exactly as it owns `Content-Length` —
/// a handler can never desync framing or force persistence.
pub fn serialize_response_conn(resp: &HttpResponse, keep_alive: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(64 + resp.body.len());
    out.extend_from_slice(
        format!(
            "HTTP/1.1 {} {}\r\n",
            resp.status,
            reason_phrase(resp.status)
        )
        .as_bytes(),
    );
    // Emit the handler's headers, but never the framing headers the writer owns —
    // so a handler cannot desync Content-Length or force keep-alive.
    for (name, value) in &resp.headers {
        let lname = name.to_ascii_lowercase();
        if matches!(
            lname.as_str(),
            "content-length" | "connection" | "transfer-encoding"
        ) {
            continue;
        }
        out.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
    }
    out.extend_from_slice(format!("Content-Length: {}\r\n", resp.body.len()).as_bytes());
    out.extend_from_slice(if keep_alive {
        b"Connection: keep-alive\r\n\r\n".as_slice()
    } else {
        b"Connection: close\r\n\r\n".as_slice()
    });
    out.extend_from_slice(&resp.body);
    out
}

/// Serialize a request to bytes: request line (method, percent-encoded path and
/// query, `HTTP/1.1`), the request's headers (minus the writer-owned framing
/// ones), an accurate `Content-Length`, `Connection: close`, the blank line,
/// then the body — the exact client-side mirror of [`serialize_response`].
///
/// `Host` rides in `headers` like any other header ([`HttpRequest`] has no
/// separate host field): callers building a client request put `host` in the
/// map and the writer emits it verbatim — it never synthesizes one. `path` and
/// `query` are the *decoded* forms ([`parse_request`]'s output); the writer
/// percent-encodes them (RFC 3986 unreserved set, `/` kept literal in the
/// path), so `parse_request(&serialize_request(req))` reconstructs them
/// exactly.
pub fn serialize_request(req: &HttpRequest) -> Vec<u8> {
    let mut target = percent_encode(&req.path, true);
    if !req.query.is_empty() {
        target.push('?');
        let mut first = true;
        for (k, v) in &req.query {
            if !first {
                target.push('&');
            }
            first = false;
            target.push_str(&percent_encode(k, false));
            target.push('=');
            target.push_str(&percent_encode(v, false));
        }
    }
    let mut out = Vec::with_capacity(64 + req.body.len());
    out.extend_from_slice(format!("{} {} HTTP/1.1\r\n", req.method, target).as_bytes());
    // Emit the caller's headers, but never the framing headers the writer owns —
    // so a caller cannot desync Content-Length or force keep-alive.
    for (name, value) in &req.headers {
        let lname = name.to_ascii_lowercase();
        if matches!(
            lname.as_str(),
            "content-length" | "connection" | "transfer-encoding"
        ) {
            continue;
        }
        out.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
    }
    out.extend_from_slice(format!("Content-Length: {}\r\n", req.body.len()).as_bytes());
    out.extend_from_slice(b"Connection: close\r\n\r\n");
    out.extend_from_slice(&req.body);
    out
}

/// Parse a complete HTTP/1.1 response from `bytes` — the client-side mirror of
/// [`parse_request`]: same [`ParseError::Incomplete`] retry contract, same
/// [`MAX_BODY`]/[`MAX_HEADERS`] caps, same lowercased header keys, same
/// `Transfer-Encoding` rejection.
///
/// Framing symmetry decision: [`serialize_response`] **always** emits an
/// accurate `Content-Length`, so this parser has no read-to-end-of-input mode —
/// a missing `Content-Length` means an empty body (the exact mirror of
/// [`parse_request`]'s default of 0). That keeps `Incomplete` meaningful: torn
/// input is always distinguishable from a complete message rather than guessed
/// at EOF.
///
/// Response-specific strictness (the client consumes bytes from arbitrary
/// servers, so mis-framing here is the smuggling hazard): the status code must
/// be exactly three ASCII digits in 100–599 (the reason phrase may be empty or
/// absent and is dropped — the write side derives it from the code), repeated
/// `Content-Length` headers must agree (RFC 9110 §8.6), and a stray CR or LF
/// inside the status line or a header line is rejected.
pub fn parse_response(bytes: &[u8]) -> Result<HttpResponse, ParseError> {
    // Locate the blank line that ends the header block. Until it arrives we do
    // not have a parseable response — but a peer that never sends it must not
    // be buffered without bound.
    let header_end = match find_subslice(bytes, b"\r\n\r\n") {
        Some(i) => i,
        None => {
            return if bytes.len() > MAX_HEADERS {
                Err(ParseError::HeadersTooLarge)
            } else {
                Err(ParseError::Incomplete)
            };
        }
    };
    if header_end > MAX_HEADERS {
        return Err(ParseError::HeadersTooLarge);
    }
    let body_start = header_end + 4;

    let head = std::str::from_utf8(&bytes[..header_end])
        .map_err(|_| ParseError::BadResponse("non-UTF-8 header block".to_string()))?;
    let mut lines = head.split("\r\n");

    // Status line: `HTTP-version SP 3DIGIT [SP reason-phrase]`. The reason may
    // contain spaces; it carries no information (the write side derives it from
    // the code), so it is validated for shape and dropped.
    let status_line = lines.next().unwrap_or("");
    if status_line.contains('\r') || status_line.contains('\n') {
        return Err(ParseError::BadResponse(format!(
            "stray CR/LF in status line: {status_line:?}"
        )));
    }
    let mut parts = status_line.splitn(3, ' ');
    let version = parts.next().unwrap_or("");
    if !version.starts_with("HTTP/") {
        return Err(ParseError::BadResponse(format!(
            "unsupported version: {version:?}"
        )));
    }
    let code = parts.next().ok_or_else(|| {
        ParseError::BadResponse(format!("malformed status line: {status_line:?}"))
    })?;
    if code.len() != 3 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ParseError::BadResponse(format!(
            "malformed status code: {code:?}"
        )));
    }
    let digits = code.as_bytes();
    let status = (digits[0] - b'0') as u16 * 100
        + (digits[1] - b'0') as u16 * 10
        + (digits[2] - b'0') as u16;
    if !(100..=599).contains(&status) {
        return Err(ParseError::BadResponse(format!(
            "status outside 100-599: {status}"
        )));
    }

    // Headers: lowercase names (case-insensitive per RFC 7230), split on the
    // first colon only so values may contain colons (e.g. a URL). Repeated
    // field lines COMMA-JOIN in arrival order (RFC 9110 §5.2), mirroring
    // parse_request — except Content-Length, where repeats must agree
    // (RFC 9110 §8.6; differing values are the classic smuggling vector, a
    // hard reject) and collapse to one (a joined "2, 2" would not parse).
    let mut headers: BTreeMap<String, String> = BTreeMap::new();
    for line in lines.filter(|l| !l.is_empty()) {
        if line.contains('\r') || line.contains('\n') {
            return Err(ParseError::BadResponse(format!(
                "stray CR/LF in header: {line:?}"
            )));
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| ParseError::BadResponse(format!("malformed header: {line:?}")))?;
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim().to_string();
        match headers.entry(name) {
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert(value);
            }
            std::collections::btree_map::Entry::Occupied(mut slot) => {
                if slot.key() == "content-length" {
                    if *slot.get() != value {
                        return Err(ParseError::BadResponse(format!(
                            "conflicting Content-Length: {:?} vs {value:?}",
                            slot.get()
                        )));
                    }
                } else {
                    let joined = format!("{}, {value}", slot.get());
                    slot.insert(joined);
                }
            }
        }
    }

    // Reject any non-identity transfer coding rather than mishandle it.
    if let Some(te) = headers.get("transfer-encoding") {
        if te
            .split(',')
            .any(|t| !t.trim().eq_ignore_ascii_case("identity"))
        {
            return Err(ParseError::UnsupportedTransferEncoding);
        }
    }

    // Body is framed by Content-Length (default 0), bounded by MAX_BODY.
    let content_length = match headers.get("content-length") {
        Some(cl) => cl
            .trim()
            .parse::<usize>()
            .map_err(|_| ParseError::BadResponse(format!("bad Content-Length: {cl:?}")))?,
        None => 0,
    };
    if content_length > MAX_BODY {
        return Err(ParseError::BodyTooLarge);
    }
    if bytes.len() < body_start + content_length {
        return Err(ParseError::Incomplete);
    }
    let body = bytes[body_start..body_start + content_length].to_vec();

    Ok(HttpResponse {
        status,
        headers,
        body,
    })
}

/// Whether a handler-supplied response header may pass to the wire — THE ONE
/// law both execution tiers (the interpreter bridge and AOT-emitted code) apply
/// before merging a LOGOS `Response.headers` map, so they can never drift:
/// the name must be non-empty, must not be writer-owned framing or hop-by-hop
/// (`connection`, `content-length`, `transfer-encoding`, `keep-alive`,
/// `upgrade`), must not be `content-type` (the `Response.contentType` field is
/// authoritative), and neither name nor value may contain CR, LF, or NUL
/// (response splitting). The typed surface (`withHeader` in `http.lg`) rejects
/// these with an `Err` up front; this filter is the defense-in-depth for maps
/// poked directly.
pub fn response_header_allowed(name: &str, value: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let lname = name.to_ascii_lowercase();
    if matches!(
        lname.as_str(),
        "connection"
            | "content-length"
            | "transfer-encoding"
            | "keep-alive"
            | "upgrade"
            | "content-type"
    ) {
        return false;
    }
    let clean = |s: &str| !s.bytes().any(|b| matches!(b, b'\r' | b'\n' | 0));
    clean(name) && clean(value)
}

/// The `Content-Type` actually emitted for a handler's `contentType` field:
/// the field verbatim when it is wire-safe, else the `text/plain` default —
/// so a CR/LF smuggled into `contentType` can never split the response. Both
/// execution tiers route the field through this, keeping them byte-identical.
pub fn safe_content_type(ct: &str) -> &str {
    if !ct.is_empty() && !ct.bytes().any(|b| matches!(b, b'\r' | b'\n' | 0)) {
        ct
    } else {
        "text/plain"
    }
}

/// Standard reason phrase for a status code; empty for codes we don't name.
fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        411 => "Length Required",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Entity",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        503 => "Service Unavailable",
        _ => "",
    }
}

/// Percent-encode `s` for the request target: every byte outside the RFC 3986
/// unreserved set (`ALPHA / DIGIT / "-" / "." / "_" / "~"`, plus `/` when
/// `keep_slash` — path segments keep their separators) becomes `%XX`, so
/// [`percent_decode`] recovers the original bytes exactly (`+` is encoded, so
/// the query's plus-as-space decode cannot corrupt a literal plus).
fn percent_encode(s: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        let keep = b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'.' | b'_' | b'~')
            || (keep_slash && b == b'/');
        if keep {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Percent-decode `s`. When `plus_as_space`, a `+` maps to a space (the
/// `application/x-www-form-urlencoded` convention for query components). A `%`
/// not followed by two hex digits is left literal.
fn percent_decode(s: &str, plus_as_space: bool) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => match (hex_val(b[i + 1]), hex_val(b[i + 2])) {
                (Some(hi), Some(lo)) => {
                    out.push(hi * 16 + lo);
                    i += 3;
                }
                _ => {
                    out.push(b'%');
                    i += 1;
                }
            },
            b'+' if plus_as_space => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Index of the first occurrence of `needle` in `hay`, if any.
fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_system/http/wire/tests.rs"]
mod tests;
