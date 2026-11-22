//! ★ LOCK — s3.lg signs S3 requests with AWS Signature Version 4, in LOGOS. ★
//!
//! Every step is pinned to the worked "GET Object" example of Amazon's Signature Version 4 documentation
//! (access key `AKIAIOSFODNN7EXAMPLE`, 2013-05-24, `us-east-1`, `examplebucket`, `GET /test.txt` with a
//! `Range`): the canonical request's hash and the `Authorization` header, on every interpreting tier.
//! The date stamp, the path encoding and the request a `s3Put` sends (against a local store that records
//! it) are locked beside them.
//!
//! YOU DO NOT GET TO WEAKEN THIS LOCK TO MAKE IT PASS.
#![allow(dead_code)]
mod common;
use common::assert_all_tiers;
use std::io::{Read, Write};
use std::time::Duration;

const EMPTY_SHA: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

fn example_program() -> String {
    format!(
        "## Main\n\
         Let store be a new S3Store with endpoint \"https://examplebucket.s3.amazonaws.com\" and region \"us-east-1\" and bucket \"examplebucket\" and accessKey \"AKIAIOSFODNN7EXAMPLE\" and secretKey \"wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY\".\n\
         Let mutable hs be a new Map of Text to Text.\n\
         Set item \"Host\" of hs to \"examplebucket.s3.amazonaws.com\".\n\
         Set item \"Range\" of hs to \"bytes=0-9\".\n\
         Set item \"x-amz-content-sha256\" of hs to \"{EMPTY_SHA}\".\n\
         Set item \"x-amz-date\" of hs to \"20130524T000000Z\".\n\
         Show s3HashHex(s3CanonicalRequest(\"GET\", \"/test.txt\", \"\", hs, \"{EMPTY_SHA}\")).\n\
         Show s3Authorization(store, \"GET\", \"/test.txt\", \"\", hs, \"{EMPTY_SHA}\", \"20130524T000000Z\").\n"
    )
}

#[test]
fn the_documented_get_object_example_signs_exactly() {
    assert_all_tiers(
        &example_program(),
        "7344ae5b7ee6c3e7e6b0fe0640412a37625d1fbfff95c48bbb2dc43964946972\n\
         AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, \
         SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, \
         Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41",
    );
}

#[test]
fn the_date_stamp_is_the_utc_civil_time() {
    assert_all_tiers(
        "## Main\nShow s3AmzDate(0).\nShow s3AmzDate(1369353600000).\nShow s3AmzDate(951782400000).\nShow s3AmzDate(1763856000123).\nShow s3AmzDate(1735689599999).\n",
        "19700101T000000Z\n20130524T000000Z\n20000229T000000Z\n20251123T000000Z\n20241231T235959Z",
    );
}

#[test]
fn an_object_key_is_encoded_segment_by_segment() {
    assert_all_tiers(
        "## Main\nShow s3EncodePath(\"/bucket/trail/OP001 attempt#1.json\").\nShow s3EncodePath(\"/b/a~b-c_d.e\").\n",
        "/bucket/trail/OP001%20attempt%231.json\n/b/a~b-c_d.e",
    );
}

#[test]
fn a_put_sends_a_signed_path_style_request() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        if let Ok((mut s, _)) = listener.accept() {
            s.set_read_timeout(Some(Duration::from_secs(10))).ok();
            let mut buf = Vec::new();
            let mut byte = [0u8; 1];
            while !buf.ends_with(b"\r\n\r\n") && s.read(&mut byte).unwrap_or(0) == 1 {
                buf.push(byte[0]);
            }
            let mut body = [0u8; 5];
            let _ = s.read_exact(&mut body);
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nETag: \"x\"\r\n\r\n");
            let _ = tx.send(String::from_utf8_lossy(&buf).to_string() + &String::from_utf8_lossy(&body));
        }
    });
    let src = format!(
        "## Main\n\
         Let store be a new S3Store with endpoint \"http://127.0.0.1:{port}\" and region \"auto\" and bucket \"macsad\" and accessKey \"AK\" and secretKey \"SK\".\n\
         Inspect s3Put(store, \"trail/OP001.jsonl\", \"hello\", \"application/x-ndjson\", 10000):\n\
         \x20   When Ok (r):\n\
         \x20       Show r's status.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n"
    );
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let out = rt.block_on(async { tokio::time::timeout(Duration::from_secs(30), logicaffeine_compile::interpret_for_ui(&src)).await.expect("no hang") });
    assert_eq!(out.error, None);
    assert_eq!(out.lines.join("\n").trim(), "200");
    let req = rx.recv_timeout(Duration::from_secs(10)).expect("the store saw the request");
    assert!(req.starts_with("PUT /macsad/trail/OP001.jsonl HTTP/1.1\r\n"), "{req}");
    for want in ["x-amz-content-sha256: UNSIGNED-PAYLOAD", "Content-Type: application/x-ndjson", "Content-Length: 5"] {
        assert!(req.contains(want), "missing `{want}`:\n{req}");
    }
    let auth = req.lines().find(|l| l.starts_with("Authorization: ")).expect("signed");
    assert!(auth.starts_with("Authorization: AWS4-HMAC-SHA256 Credential=AK/"), "{auth}");
    assert!(auth.contains("/auto/s3/aws4_request, SignedHeaders=host;x-amz-content-sha256;x-amz-date, Signature="), "{auth}");
    let sig = auth.rsplit("Signature=").next().unwrap();
    assert_eq!(sig.len(), 64, "a hex SHA-256 signature: {auth}");
    assert!(req.ends_with("hello"), "the body follows:\n{req}");
}
