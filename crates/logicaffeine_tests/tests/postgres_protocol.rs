//! ════════════════════════════════════════════════════════════════════════════════════════════
//! A3 — POSTGRES PROTOCOL LOCK: the pure-LOGOS postgres.lg client speaks the v3 wire protocol
//! against SCRIPTED loopback servers, byte-identically on the TREE-WALKER (`interpret_for_ui`)
//! and the BYTECODE VM (`run_vm_io_async`). Every frontend frame the client emits is asserted
//! byte-for-byte against checked-in fixtures (tests/assets/pg/*.hex) or against the
//! postgres-protocol reference encoder; every backend frame is canned straight from the docs'
//! "Message Formats". A REAL protocol implementation (the pgwire crate) hosts the interop test.
//!
//!  ⚠️  YOU DO NOT GET TO WEAKEN THIS LOCK TO MAKE IT PASS.  ⚠️
//!  A tier divergence means the tcp/interpreter tiers drifted; a byte divergence means the
//!  client stopped speaking PostgreSQL. Fix the module or the tier — never the fixtures.
//!  Strictly monotone: add shapes, never remove them.
//! ════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::time::Duration;

use logicaffeine_compile::{interpret_for_ui, run_vm_io_async};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

// ---------------------------------------------------------------------------
// Fixtures and frame builders (server side = straight transliteration of the
// docs' Message Formats; client-side expectations = checked-in fixtures)
// ---------------------------------------------------------------------------

fn hexdec(s: &str) -> Vec<u8> {
    let s = s.trim();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("valid hex"))
        .collect()
}

fn fixture_bytes(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/assets/pg")
        .join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("missing fixture {}: {e}", path.display()));
    hexdec(&text)
}

/// A tagged backend message: tag byte, Int32 length (self + body), body.
fn tagged(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    out.extend_from_slice(&((body.len() as i32 + 4).to_be_bytes()));
    out.extend_from_slice(body);
    out
}

fn cstr(s: &str) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    v.push(0);
    v
}

/// AuthenticationRequest 'R' with the given code and trailing data.
fn auth_request(code: i32, extra: &[u8]) -> Vec<u8> {
    let mut body = code.to_be_bytes().to_vec();
    body.extend_from_slice(extra);
    tagged(b'R', &body)
}

/// RowDescription 'T' for (name, type-oid) columns, text format.
fn row_description(cols: &[(&str, i32)]) -> Vec<u8> {
    let mut body = (cols.len() as i16).to_be_bytes().to_vec();
    for (name, oid) in cols {
        body.extend_from_slice(&cstr(name));
        body.extend_from_slice(&0i32.to_be_bytes()); // table oid
        body.extend_from_slice(&0i16.to_be_bytes()); // column attr number
        body.extend_from_slice(&oid.to_be_bytes()); // type oid
        body.extend_from_slice(&(-1i16).to_be_bytes()); // typlen
        body.extend_from_slice(&(-1i32).to_be_bytes()); // atttypmod
        body.extend_from_slice(&0i16.to_be_bytes()); // format: text
    }
    tagged(b'T', &body)
}

/// DataRow 'D': None = NULL (length -1).
fn data_row(cells: &[Option<&str>]) -> Vec<u8> {
    let mut body = (cells.len() as i16).to_be_bytes().to_vec();
    for cell in cells {
        match cell {
            None => body.extend_from_slice(&(-1i32).to_be_bytes()),
            Some(v) => {
                body.extend_from_slice(&(v.len() as i32).to_be_bytes());
                body.extend_from_slice(v.as_bytes());
            }
        }
    }
    tagged(b'D', &body)
}

fn command_complete(tag_text: &str) -> Vec<u8> {
    tagged(b'C', &cstr(tag_text))
}

fn ready_for_query() -> Vec<u8> {
    tagged(b'Z', b"I")
}

fn parameter_status(key: &str, value: &str) -> Vec<u8> {
    let mut body = cstr(key);
    body.extend_from_slice(&cstr(value));
    tagged(b'S', &body)
}

fn backend_key_data() -> Vec<u8> {
    let mut body = 18i32.to_be_bytes().to_vec();
    body.extend_from_slice(&52i32.to_be_bytes());
    tagged(b'K', &body)
}

/// ErrorResponse/NoticeResponse: (field byte, cstring) pairs + terminator.
fn fields_message(tag: u8, fields: &[(u8, &str)]) -> Vec<u8> {
    let mut body = Vec::new();
    for (ft, v) in fields {
        body.push(*ft);
        body.extend_from_slice(&cstr(v));
    }
    body.push(0);
    tagged(tag, &body)
}

fn error_syntax() -> Vec<u8> {
    fields_message(
        b'E',
        &[
            (b'S', "ERROR"),
            (b'V', "ERROR"),
            (b'C', "42601"),
            (b'M', "syntax error at or near \"BOGUS\""),
        ],
    )
}

/// The reference encoding of a Query('Q') frame, via the postgres-protocol
/// crate — used for Expect steps beyond the named fixtures.
fn query_frame(sql: &str) -> Vec<u8> {
    let mut buf = bytes::BytesMut::new();
    postgres_protocol::message::frontend::query(sql, &mut buf).expect("query encodes");
    buf.to_vec()
}

// ---------------------------------------------------------------------------
// The scripted server
// ---------------------------------------------------------------------------

#[derive(Clone)]
enum Step {
    /// Read exactly these bytes from the client and record what arrived.
    Expect(Vec<u8>),
    /// Write these bytes in one burst.
    Send(Vec<u8>),
    /// Write these bytes ONE AT A TIME with small sleeps — TCP has no message
    /// boundaries, so the client's framing must reassemble.
    SendSlow(Vec<u8>),
    /// Shut the write half down immediately (mid-message close).
    Close,
}

/// Accept ONE connection and run the script. Returns every Expect step's
/// (wanted, actual) byte pair so the test body can assert equality fail-loud.
fn spawn_script_server(
    steps: Vec<Step>,
) -> (u16, tokio::task::JoinHandle<Vec<(Vec<u8>, Vec<u8>)>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("binds");
    listener.set_nonblocking(true).expect("nonblocking");
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        let listener = TcpListener::from_std(listener).expect("tokio listener");
        let (mut s, _) = listener.accept().await.expect("client connects");
        let mut reads = Vec::new();
        for step in steps {
            match step {
                Step::Expect(want) => {
                    let mut buf = vec![0u8; want.len()];
                    s.read_exact(&mut buf)
                        .await
                        .expect("the expected client bytes arrive");
                    reads.push((want, buf));
                }
                Step::Send(bytes) => {
                    s.write_all(&bytes).await.expect("server write");
                    s.flush().await.ok();
                }
                Step::SendSlow(bytes) => {
                    for b in bytes {
                        s.write_all(&[b]).await.expect("server slow write");
                        s.flush().await.ok();
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                }
                Step::Close => {
                    s.shutdown().await.ok();
                    return reads;
                }
            }
        }
        // Give the client a beat to finish its last reads before the socket drops.
        tokio::time::sleep(Duration::from_millis(200)).await;
        reads
    });
    (port, handle)
}

async fn run_tier(
    program: &str,
    use_vm: bool,
) -> logicaffeine_compile::interpreter::InterpreterResult {
    let fut = async {
        if use_vm {
            run_vm_io_async(program).await
        } else {
            interpret_for_ui(program).await
        }
    };
    tokio::time::timeout(Duration::from_secs(30), fut)
        .await
        .expect("the program must not hang")
}

/// Run the SAME program shape on the tree-walker and the VM, each against its
/// own fresh scripted server; assert (1) every Expect step saw byte-identical
/// frames on both tiers, and (2) both tiers produced the SAME lines, equal to
/// `expected_lines`.
async fn lock_both_tiers(
    make_program: impl Fn(u16) -> String,
    steps: &[Step],
    expected_lines: &[&str],
) {
    let mut all_lines = Vec::new();
    for use_vm in [false, true] {
        let (port, server) = spawn_script_server(steps.to_vec());
        let program = make_program(port);
        let result = run_tier(&program, use_vm).await;
        assert!(
            result.error.is_none(),
            "tier vm={use_vm} errored: {:?}\nprogram:\n{program}",
            result.error
        );
        let reads = tokio::time::timeout(Duration::from_secs(30), server)
            .await
            .expect("the scripted server must finish")
            .expect("the scripted server must not panic");
        for (i, (want, got)) in reads.iter().enumerate() {
            assert_eq!(
                hex(got),
                hex(want),
                "tier vm={use_vm}: client frame {} diverged from the pinned bytes",
                i + 1
            );
        }
        all_lines.push(result.lines);
    }
    assert_eq!(
        all_lines[0], all_lines[1],
        "TIER DIVERGENCE: tree-walker and VM produced different lines"
    );
    assert_eq!(all_lines[0], expected_lines, "the agreed output drifted");
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------------------
// Programs
// ---------------------------------------------------------------------------

/// Connect, run the two-column query, exercise every typed getter (including
/// the out-of-range and wrong-type Err arms), close.
fn happy_path_program(port: u16) -> String {
    format!(
        "## Main\n\
         Let cR be pgConnect(\"postgres://alice:secret@127.0.0.1:{port}/appdb\").\n\
         Inspect cR:\n\
         \x20   When Ok (pgc):\n\
         \x20       Let qR be pgQuery(pgc, \"SELECT id, name FROM t\").\n\
         \x20       Inspect qR:\n\
         \x20           When Ok (qr):\n\
         \x20               Let colsSeq be qr's columns.\n\
         \x20               Let colsText be join(colsSeq, \",\").\n\
         \x20               Show colsText.\n\
         \x20               Show qr's typeOids.\n\
         \x20               Let rws be qr's rows.\n\
         \x20               Let nrows be length of rws.\n\
         \x20               Show nrows.\n\
         \x20               Let a1 be pgIntAt(qr, 1, 1).\n\
         \x20               Inspect a1:\n\
         \x20                   When Ok (v):\n\
         \x20                       Show v.\n\
         \x20                   When Err (e):\n\
         \x20                       Show e.\n\
         \x20               Let a2 be pgTextAt(qr, 1, 2).\n\
         \x20               Inspect a2:\n\
         \x20                   When Ok (v):\n\
         \x20                       Show v.\n\
         \x20                   When Err (e):\n\
         \x20                       Show e.\n\
         \x20               Let a3 be pgIntAt(qr, 2, 1).\n\
         \x20               Inspect a3:\n\
         \x20                   When Ok (v):\n\
         \x20                       Show v.\n\
         \x20                   When Err (e):\n\
         \x20                       Show e.\n\
         \x20               Let a4 be pgTextAt(qr, 2, 2).\n\
         \x20               Inspect a4:\n\
         \x20                   When Ok (v):\n\
         \x20                       Show v.\n\
         \x20                   When Err (e):\n\
         \x20                       Show e.\n\
         \x20               Let bad1 be pgIntAt(qr, 1, 2).\n\
         \x20               Inspect bad1:\n\
         \x20                   When Ok (v):\n\
         \x20                       Show \"unexpectedly parsed\".\n\
         \x20                   When Err (e):\n\
         \x20                       Show e.\n\
         \x20               Let bad2 be pgTextAt(qr, 3, 1).\n\
         \x20               Inspect bad2:\n\
         \x20                   When Ok (v):\n\
         \x20                       Show \"unexpectedly in range\".\n\
         \x20                   When Err (e):\n\
         \x20                       Show e.\n\
         \x20           When Err (e):\n\
         \x20               Show e.\n\
         \x20       Let closed be pgClose(pgc).\n\
         \x20       Inspect closed:\n\
         \x20           When Ok (u):\n\
         \x20               Show \"closed\".\n\
         \x20           When Err (e):\n\
         \x20               Show e.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n"
    )
}

const HAPPY_EXPECTED: &[&str] = &[
    "id,name",
    "[23, 25]",
    "2",
    "1",
    "alice",
    "2",
    "bob",
    "pg: the value 'alice' is not an Int",
    "pg: row 3 is out of range (1..2)",
    "closed",
];

fn happy_reply() -> Vec<u8> {
    let mut reply = Vec::new();
    reply.extend(row_description(&[("id", 23), ("name", 25)]));
    reply.extend(data_row(&[Some("1"), Some("alice")]));
    reply.extend(data_row(&[Some("2"), Some("bob")]));
    reply.extend(command_complete("SELECT 2"));
    reply.extend(ready_for_query());
    reply
}

/// Cleartext auth preamble: expect the pinned startup, demand a password,
/// expect the pinned cleartext 'p' frame, then AuthOk → ParameterStatus →
/// BackendKeyData → ReadyForQuery.
fn cleartext_preamble() -> Vec<Step> {
    let mut post_auth = auth_request(0, &[]);
    post_auth.extend(parameter_status("server_version", "16.0"));
    post_auth.extend(parameter_status("client_encoding", "UTF8"));
    post_auth.extend(backend_key_data());
    post_auth.extend(ready_for_query());
    vec![
        Step::Expect(fixture_bytes("startup_alice_appdb.hex")),
        Step::Send(auth_request(3, &[])),
        Step::Expect(fixture_bytes("password_cleartext_secret.hex")),
        Step::Send(post_auth),
    ]
}

/// Trust preamble: AuthenticationOk straight away (no password frame).
fn trust_preamble() -> Vec<Step> {
    let mut post_auth = auth_request(0, &[]);
    post_auth.extend(parameter_status("server_version", "16.0"));
    post_auth.extend(backend_key_data());
    post_auth.extend(ready_for_query());
    vec![
        Step::Expect(fixture_bytes("startup_alice_appdb.hex")),
        Step::Send(post_auth),
    ]
}

// ---------------------------------------------------------------------------
// The locks
// ---------------------------------------------------------------------------

/// The full happy path over cleartext auth: startup → password → query →
/// 2 columns, 2 rows, typed getters (happy AND error arms) → terminate. The
/// client's startup, password, query, and terminate frames are asserted
/// byte-for-byte against the checked-in fixtures on BOTH tiers.
#[tokio::test]
async fn cleartext_auth_and_simple_query_agree_across_tiers() {
    let mut steps = cleartext_preamble();
    steps.push(Step::Expect(fixture_bytes("query_select_id_name.hex")));
    steps.push(Step::Send(happy_reply()));
    steps.push(Step::Expect(fixture_bytes("terminate.hex")));
    lock_both_tiers(happy_path_program, &steps, HAPPY_EXPECTED).await;
}

/// TORN DELIVERY: the same happy path, but the server writes its entire reply
/// stream ONE BYTE AT A TIME with sleeps. The client's length-prefixed framing
/// must reassemble across arbitrary TCP fragmentation — identical output.
#[tokio::test]
async fn torn_delivery_reassembles_byte_by_byte() {
    let mut post_auth = auth_request(0, &[]);
    post_auth.extend(parameter_status("server_version", "16.0"));
    post_auth.extend(backend_key_data());
    post_auth.extend(ready_for_query());
    let steps = vec![
        Step::Expect(fixture_bytes("startup_alice_appdb.hex")),
        Step::SendSlow(auth_request(3, &[])),
        Step::Expect(fixture_bytes("password_cleartext_secret.hex")),
        Step::SendSlow(post_auth),
        Step::Expect(fixture_bytes("query_select_id_name.hex")),
        Step::SendSlow(happy_reply()),
        Step::Expect(fixture_bytes("terminate.hex")),
    ];
    lock_both_tiers(happy_path_program, &steps, HAPPY_EXPECTED).await;
}

/// MD5 auth: the server demands AuthenticationMD5Password with salt "qwer";
/// the client's response frame must be byte-identical to the pinned
/// md5(md5(password+user)+salt) fixture.
#[tokio::test]
async fn md5_auth_agrees_across_tiers() {
    let program = |port: u16| {
        format!(
            "## Main\n\
             Let cR be pgConnect(\"postgres://alice:secret@127.0.0.1:{port}/appdb\").\n\
             Inspect cR:\n\
             \x20   When Ok (pgc):\n\
             \x20       Show \"connected\".\n\
             \x20       Let closed be pgClose(pgc).\n\
             \x20       Inspect closed:\n\
             \x20           When Ok (u):\n\
             \x20               Show \"closed\".\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20   When Err (e):\n\
             \x20       Show e.\n"
        )
    };
    let mut post_auth = auth_request(0, &[]);
    post_auth.extend(ready_for_query());
    let steps = vec![
        Step::Expect(fixture_bytes("startup_alice_appdb.hex")),
        Step::Send(auth_request(5, b"qwer")),
        Step::Expect(fixture_bytes("password_md5_alice_secret_qwer.hex")),
        Step::Send(post_auth),
        Step::Expect(fixture_bytes("terminate.hex")),
    ];
    lock_both_tiers(program, &steps, &["connected", "closed"]).await;
}

/// An ErrorResponse mid-query surfaces as an Err carrying severity, SQLSTATE,
/// and message — and the connection stays usable: the protocol drains to
/// ReadyForQuery, so a SECOND query on the same connection still succeeds.
#[tokio::test]
async fn error_response_surfaces_and_the_connection_stays_usable() {
    let program = |port: u16| {
        format!(
            "## Main\n\
             Let cR be pgConnect(\"postgres://alice:secret@127.0.0.1:{port}/appdb\").\n\
             Inspect cR:\n\
             \x20   When Ok (pgc):\n\
             \x20       Let qR be pgQuery(pgc, \"SELECT * FROM BOGUS\").\n\
             \x20       Inspect qR:\n\
             \x20           When Ok (qr):\n\
             \x20               Show \"unexpectedly ok\".\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let q2 be pgQuery(pgc, \"SELECT 1\").\n\
             \x20       Inspect q2:\n\
             \x20           When Ok (qr):\n\
             \x20               Let v be pgIntAt(qr, 1, 1).\n\
             \x20               Inspect v:\n\
             \x20                   When Ok (n):\n\
             \x20                       Show n.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let closed be pgClose(pgc).\n\
             \x20       Show \"done\".\n\
             \x20   When Err (e):\n\
             \x20       Show e.\n"
        )
    };
    let mut error_reply = error_syntax();
    error_reply.extend(ready_for_query());
    let mut select1_reply = row_description(&[("?column?", 23)]);
    select1_reply.extend(data_row(&[Some("1")]));
    select1_reply.extend(command_complete("SELECT 1"));
    select1_reply.extend(ready_for_query());
    let mut steps = trust_preamble();
    steps.push(Step::Expect(query_frame("SELECT * FROM BOGUS")));
    steps.push(Step::Send(error_reply));
    steps.push(Step::Expect(query_frame("SELECT 1")));
    steps.push(Step::Send(select1_reply));
    steps.push(Step::Expect(fixture_bytes("terminate.hex")));
    lock_both_tiers(
        program,
        &steps,
        &[
            "ERROR 42601: syntax error at or near \"BOGUS\"",
            "1",
            "done",
        ],
    )
    .await;
}

/// A NoticeResponse interleaved between DataRows is skipped — the rows still
/// arrive intact.
#[tokio::test]
async fn notice_response_interleaved_is_skipped() {
    let mut reply = Vec::new();
    reply.extend(row_description(&[("id", 23), ("name", 25)]));
    reply.extend(data_row(&[Some("1"), Some("alice")]));
    reply.extend(fields_message(
        b'N',
        &[
            (b'S', "NOTICE"),
            (b'C', "00000"),
            (b'M', "just so you know"),
        ],
    ));
    reply.extend(data_row(&[Some("2"), Some("bob")]));
    reply.extend(command_complete("SELECT 2"));
    reply.extend(ready_for_query());
    let mut steps = cleartext_preamble();
    steps.push(Step::Expect(fixture_bytes("query_select_id_name.hex")));
    steps.push(Step::Send(reply));
    steps.push(Step::Expect(fixture_bytes("terminate.hex")));
    lock_both_tiers(happy_path_program, &steps, HAPPY_EXPECTED).await;
}

/// An empty result set: RowDescription then CommandComplete with zero DataRows
/// — columns present, zero rows, and the getters' range Errs stay honest.
#[tokio::test]
async fn empty_result_set_round_trips() {
    let program = |port: u16| {
        format!(
            "## Main\n\
             Let cR be pgConnect(\"postgres://alice:secret@127.0.0.1:{port}/appdb\").\n\
             Inspect cR:\n\
             \x20   When Ok (pgc):\n\
             \x20       Let qR be pgQuery(pgc, \"SELECT id, name FROM t\").\n\
             \x20       Inspect qR:\n\
             \x20           When Ok (qr):\n\
             \x20               Let colsSeq be qr's columns.\n\
             \x20               Let colsText be join(colsSeq, \",\").\n\
             \x20               Show colsText.\n\
             \x20               Let rws be qr's rows.\n\
             \x20               Let nrows be length of rws.\n\
             \x20               Show nrows.\n\
             \x20               Let v be pgTextAt(qr, 1, 1).\n\
             \x20               Inspect v:\n\
             \x20                   When Ok (t):\n\
             \x20                       Show \"unexpectedly in range\".\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let closed be pgClose(pgc).\n\
             \x20       Show \"done\".\n\
             \x20   When Err (e):\n\
             \x20       Show e.\n"
        )
    };
    let mut reply = row_description(&[("id", 23), ("name", 25)]);
    reply.extend(command_complete("SELECT 0"));
    reply.extend(ready_for_query());
    let mut steps = trust_preamble();
    steps.push(Step::Expect(fixture_bytes("query_select_id_name.hex")));
    steps.push(Step::Send(reply));
    steps.push(Step::Expect(fixture_bytes("terminate.hex")));
    lock_both_tiers(
        program,
        &steps,
        &["id,name", "0", "pg: row 1 is out of range (1..0)", "done"],
    )
    .await;
}

/// A NULL column (DataRow length -1) surfaces as "" in this version — shown
/// delimited so the empty is observable — while its neighbors arrive intact.
#[tokio::test]
async fn null_column_arrives_as_empty_text() {
    let program = |port: u16| {
        format!(
            "## Main\n\
             Let cR be pgConnect(\"postgres://alice:secret@127.0.0.1:{port}/appdb\").\n\
             Inspect cR:\n\
             \x20   When Ok (pgc):\n\
             \x20       Let qR be pgQuery(pgc, \"SELECT id, name FROM t\").\n\
             \x20       Inspect qR:\n\
             \x20           When Ok (qr):\n\
             \x20               Let c1 be pgTextAt(qr, 1, 1).\n\
             \x20               Inspect c1:\n\
             \x20                   When Ok (v):\n\
             \x20                       Show \"<{{v}}>\".\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let c2 be pgTextAt(qr, 1, 2).\n\
             \x20               Inspect c2:\n\
             \x20                   When Ok (v):\n\
             \x20                       Show \"<{{v}}>\".\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let closed be pgClose(pgc).\n\
             \x20       Show \"done\".\n\
             \x20   When Err (e):\n\
             \x20       Show e.\n"
        )
    };
    let mut reply = row_description(&[("id", 23), ("name", 25)]);
    reply.extend(data_row(&[None, Some("ghost")]));
    reply.extend(command_complete("SELECT 1"));
    reply.extend(ready_for_query());
    let mut steps = trust_preamble();
    steps.push(Step::Expect(fixture_bytes("query_select_id_name.hex")));
    steps.push(Step::Send(reply));
    steps.push(Step::Expect(fixture_bytes("terminate.hex")));
    lock_both_tiers(program, &steps, &["<>", "<ghost>", "done"]).await;
}

/// An unknown message tag is a CLEAN Err — never a hang, never a trap.
#[tokio::test]
async fn unknown_message_tag_is_a_clean_err() {
    let program = |port: u16| {
        format!(
            "## Main\n\
             Let cR be pgConnect(\"postgres://alice:secret@127.0.0.1:{port}/appdb\").\n\
             Inspect cR:\n\
             \x20   When Ok (pgc):\n\
             \x20       Let qR be pgQuery(pgc, \"SELECT 1\").\n\
             \x20       Inspect qR:\n\
             \x20           When Ok (qr):\n\
             \x20               Show \"unexpectedly ok\".\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let closed be pgClose(pgc).\n\
             \x20       Show \"done\".\n\
             \x20   When Err (e):\n\
             \x20       Show e.\n"
        )
    };
    let mut steps = trust_preamble();
    steps.push(Step::Expect(query_frame("SELECT 1")));
    steps.push(Step::Send(tagged(b'~', &[])));
    lock_both_tiers(
        program,
        &steps,
        &["pgQuery: unexpected message tag 126", "done"],
    )
    .await;
}

/// The server closing MID-MESSAGE surfaces the tcp torn-frame Err: three bytes
/// of a RowDescription arrive (tag + 2 of the 4 length bytes), then FIN.
#[tokio::test]
async fn server_close_mid_message_surfaces_the_torn_frame() {
    let program = |port: u16| {
        format!(
            "## Main\n\
             Let cR be pgConnect(\"postgres://alice:secret@127.0.0.1:{port}/appdb\").\n\
             Inspect cR:\n\
             \x20   When Ok (pgc):\n\
             \x20       Let qR be pgQuery(pgc, \"SELECT 1\").\n\
             \x20       Inspect qR:\n\
             \x20           When Ok (qr):\n\
             \x20               Show \"unexpectedly ok\".\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let closed be pgClose(pgc).\n\
             \x20       Show \"done\".\n\
             \x20   When Err (e):\n\
             \x20       Show e.\n"
        )
    };
    let partial = row_description(&[("id", 23)])[..3].to_vec();
    let mut steps = trust_preamble();
    steps.push(Step::Expect(query_frame("SELECT 1")));
    steps.push(Step::Send(partial));
    steps.push(Step::Close);
    lock_both_tiers(
        program,
        &steps,
        &["pgQuery: tcp: connection closed after 2 of 4 bytes", "done"],
    )
    .await;
}

/// pgExecute parses rows-affected from the CommandComplete tag: "INSERT 0 3"
/// → 3, and a tag with no count ("CREATE TABLE") → 0.
#[tokio::test]
async fn execute_rows_affected_agrees_across_tiers() {
    let program = |port: u16| {
        format!(
            "## Main\n\
             Let cR be pgConnect(\"postgres://alice:secret@127.0.0.1:{port}/appdb\").\n\
             Inspect cR:\n\
             \x20   When Ok (pgc):\n\
             \x20       Let e1 be pgExecute(pgc, \"CREATE TABLE t (a int)\").\n\
             \x20       Inspect e1:\n\
             \x20           When Ok (n):\n\
             \x20               Show n.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let e2 be pgExecute(pgc, \"INSERT INTO t SELECT 1\").\n\
             \x20       Inspect e2:\n\
             \x20           When Ok (n):\n\
             \x20               Show n.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let closed be pgClose(pgc).\n\
             \x20       Show \"done\".\n\
             \x20   When Err (e):\n\
             \x20       Show e.\n"
        )
    };
    let mut create_reply = command_complete("CREATE TABLE");
    create_reply.extend(ready_for_query());
    let mut insert_reply = command_complete("INSERT 0 3");
    insert_reply.extend(ready_for_query());
    let mut steps = trust_preamble();
    steps.push(Step::Expect(query_frame("CREATE TABLE t (a int)")));
    steps.push(Step::Send(create_reply));
    steps.push(Step::Expect(query_frame("INSERT INTO t SELECT 1")));
    steps.push(Step::Send(insert_reply));
    steps.push(Step::Expect(fixture_bytes("terminate.hex")));
    lock_both_tiers(program, &steps, &["0", "3", "done"]).await;
}

// ═══════════════════════════════════════════════════════════════════════════
// G3 — EXTENDED QUERY PROTOCOL conversations against scripted servers, on
// BOTH tiers, every client frame asserted byte-for-byte against the fixtures.
// Backend frames are straight from the docs' Message Formats + the postgres
// source (protocol.h:38-58 for the tags).
// ═══════════════════════════════════════════════════════════════════════════

/// ParseComplete '1', BindComplete '2', NoData 'n', PortalSuspended 's' —
/// empty-body acknowledgements (protocol.h:38-57).
fn empty_tagged(tag: u8) -> Vec<u8> {
    tagged(tag, &[])
}

/// ParameterDescription 't': Int16 count + Int32 type OIDs (docs "Message
/// Formats"; sent by exec_describe_statement_message, postgres.c:2726).
fn parameter_description(oids: &[i32]) -> Vec<u8> {
    let mut body = (oids.len() as i16).to_be_bytes().to_vec();
    for oid in oids {
        body.extend_from_slice(&oid.to_be_bytes());
    }
    tagged(b't', &body)
}

/// The exact bytes of the pgPrepare flight: Parse + Describe('S') + Sync.
fn prepare_flight_bytes() -> Vec<u8> {
    let mut flight = fixture_bytes("parse_named_s1.hex");
    flight.extend(fixture_bytes("describe_statement_s1.hex"));
    flight.extend(fixture_bytes("sync.hex"));
    flight
}

/// The exact bytes of a pgExecPrepared flight for statement "s1":
/// Bind + Describe('P') + Execute + Sync.
fn exec_flight_bytes(bind_fixture: &str) -> Vec<u8> {
    let mut flight = fixture_bytes(bind_fixture);
    flight.extend(fixture_bytes("describe_portal_unnamed.hex"));
    flight.extend(fixture_bytes("execute_unnamed_all.hex"));
    flight.extend(fixture_bytes("sync.hex"));
    flight
}

/// pgPrepare captures the ParameterDescription OIDs and the RowDescription
/// columns from Parse+Describe('S')+Sync — the client flight is byte-exact
/// against the three fixtures, on both tiers.
#[tokio::test]
async fn prepare_describe_captures_params_and_columns() {
    let program = |port: u16| {
        format!(
            "## Main\n\
             Let cR be pgConnect(\"postgres://alice:secret@127.0.0.1:{port}/appdb\").\n\
             Inspect cR:\n\
             \x20   When Ok (pgc):\n\
             \x20       Let pR be pgPrepare(pgc, \"s1\", \"SELECT $1::int4, $2::text\").\n\
             \x20       Inspect pR:\n\
             \x20           When Ok (prep):\n\
             \x20               Show prep's stmtName.\n\
             \x20               Show prep's paramOids.\n\
             \x20               Let colsSeq be prep's columns.\n\
             \x20               Let colsText be join(colsSeq, \",\").\n\
             \x20               Show colsText.\n\
             \x20               Show prep's typeOids.\n\
             \x20               Show prep's hasRows.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let closed be pgClose(pgc).\n\
             \x20       Show \"done\".\n\
             \x20   When Err (e):\n\
             \x20       Show e.\n"
        )
    };
    let mut reply = empty_tagged(b'1');
    reply.extend(parameter_description(&[23, 25]));
    reply.extend(row_description(&[("int4", 23), ("text", 25)]));
    reply.extend(ready_for_query());
    let mut steps = trust_preamble();
    steps.push(Step::Expect(prepare_flight_bytes()));
    steps.push(Step::Send(reply));
    steps.push(Step::Expect(fixture_bytes("terminate.hex")));
    lock_both_tiers(
        program,
        &steps,
        &["s1", "[23, 25]", "int4,text", "[23, 25]", "true", "done"],
    )
    .await;
}

/// Describing DML without RETURNING answers NoData 'n' — hasRows false, no
/// columns, and the declared param OIDs still arrive.
#[tokio::test]
async fn prepare_nodata_statement_has_no_rows() {
    let program = |port: u16| {
        format!(
            "## Main\n\
             Let cR be pgConnect(\"postgres://alice:secret@127.0.0.1:{port}/appdb\").\n\
             Inspect cR:\n\
             \x20   When Ok (pgc):\n\
             \x20       Let pR be pgPrepare(pgc, \"s1\", \"SELECT $1::int4, $2::text\").\n\
             \x20       Inspect pR:\n\
             \x20           When Ok (prep):\n\
             \x20               Show prep's paramOids.\n\
             \x20               Let colsSeq be prep's columns.\n\
             \x20               Show length of colsSeq.\n\
             \x20               Show prep's hasRows.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let closed be pgClose(pgc).\n\
             \x20       Show \"done\".\n\
             \x20   When Err (e):\n\
             \x20       Show e.\n"
        )
    };
    let mut reply = empty_tagged(b'1');
    reply.extend(parameter_description(&[23, 25]));
    reply.extend(empty_tagged(b'n'));
    reply.extend(ready_for_query());
    let mut steps = trust_preamble();
    steps.push(Step::Expect(prepare_flight_bytes()));
    steps.push(Step::Send(reply));
    steps.push(Step::Expect(fixture_bytes("terminate.hex")));
    lock_both_tiers(program, &steps, &["[23, 25]", "0", "false", "done"]).await;
}

/// The full prepared round-trip: prepare, then execute with two text params —
/// BindComplete '2' flows, DataRows land with an all-false mask, and the
/// CommandComplete tag reports rows affected through pgRowsAffected. Client
/// frames byte-exact on both tiers.
#[tokio::test]
async fn exec_prepared_binds_and_collects() {
    let program = |port: u16| {
        format!(
            "## Main\n\
             Let cR be pgConnect(\"postgres://alice:secret@127.0.0.1:{port}/appdb\").\n\
             Inspect cR:\n\
             \x20   When Ok (pgc):\n\
             \x20       Let pR be pgPrepare(pgc, \"s1\", \"SELECT $1::int4, $2::text\").\n\
             \x20       Inspect pR:\n\
             \x20           When Ok (prep):\n\
             \x20               Show prep's hasRows.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let ps: Seq of Text be [\"42\", \"hello\"].\n\
             \x20       Let qR be pgExecPrepared(pgc, \"s1\", ps).\n\
             \x20       Inspect qR:\n\
             \x20           When Ok (qr):\n\
             \x20               Let colsSeq be qr's columns.\n\
             \x20               Let colsText be join(colsSeq, \",\").\n\
             \x20               Show colsText.\n\
             \x20               Let rws be qr's rows.\n\
             \x20               Show length of rws.\n\
             \x20               Let v1 be pgIntAt(qr, 1, 1).\n\
             \x20               Inspect v1:\n\
             \x20                   When Ok (v):\n\
             \x20                       Show v.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let v2 be pgTextAt(qr, 1, 2).\n\
             \x20               Inspect v2:\n\
             \x20                   When Ok (v):\n\
             \x20                       Show v.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let n1 be pgIsNullAt(qr, 1, 1).\n\
             \x20               Inspect n1:\n\
             \x20                   When Ok (v):\n\
             \x20                       Show v.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let tagT be qr's cmdTag.\n\
             \x20               Show tagT.\n\
             \x20               Show pgRowsAffected(tagT).\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let closed be pgClose(pgc).\n\
             \x20       Show \"done\".\n\
             \x20   When Err (e):\n\
             \x20       Show e.\n"
        )
    };
    let mut prep_reply = empty_tagged(b'1');
    prep_reply.extend(parameter_description(&[23, 25]));
    prep_reply.extend(row_description(&[("int4", 23), ("text", 25)]));
    prep_reply.extend(ready_for_query());
    let mut exec_reply = empty_tagged(b'2');
    exec_reply.extend(row_description(&[("int4", 23), ("text", 25)]));
    exec_reply.extend(data_row(&[Some("42"), Some("hello")]));
    exec_reply.extend(command_complete("SELECT 1"));
    exec_reply.extend(ready_for_query());
    let mut steps = trust_preamble();
    steps.push(Step::Expect(prepare_flight_bytes()));
    steps.push(Step::Send(prep_reply));
    steps.push(Step::Expect(exec_flight_bytes(
        "bind_s1_params_42_hello.hex",
    )));
    steps.push(Step::Send(exec_reply));
    steps.push(Step::Expect(fixture_bytes("terminate.hex")));
    lock_both_tiers(
        program,
        &steps,
        &[
            "true",
            "int4,text",
            "1",
            "42",
            "hello",
            "false",
            "SELECT 1",
            "1",
            "done",
        ],
    )
    .await;
}

/// NULL round-trip BOTH directions: pgExecPreparedN's nulls mask sends the
/// second param as Int32 -1 (fixture-exact), and a NULL result cell comes
/// back as mask true / Opt none while the legacy getter still projects "".
/// Empty string stays distinct from NULL throughout.
#[tokio::test]
async fn null_param_and_null_cell_round_trip() {
    let program = |port: u16| {
        format!(
            "## Main\n\
             Let cR be pgConnect(\"postgres://alice:secret@127.0.0.1:{port}/appdb\").\n\
             Inspect cR:\n\
             \x20   When Ok (pgc):\n\
             \x20       Let ps: Seq of Text be [\"42\", \"\"].\n\
             \x20       Let ns: Seq of Bool be [false, true].\n\
             \x20       Let qR be pgExecPreparedN(pgc, \"s1\", ps, ns).\n\
             \x20       Inspect qR:\n\
             \x20           When Ok (qr):\n\
             \x20               Let n1 be pgIsNullAt(qr, 1, 1).\n\
             \x20               Inspect n1:\n\
             \x20                   When Ok (v):\n\
             \x20                       Show v.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let n2 be pgIsNullAt(qr, 1, 2).\n\
             \x20               Inspect n2:\n\
             \x20                   When Ok (v):\n\
             \x20                       Show v.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let o1 be pgTextAtOpt(qr, 1, 1).\n\
             \x20               Inspect o1:\n\
             \x20                   When Ok (ov):\n\
             \x20                       Inspect ov:\n\
             \x20                           When Some (v):\n\
             \x20                               Show \"some <{{v}}>\".\n\
             \x20                           When None:\n\
             \x20                               Show \"none\".\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let o2 be pgTextAtOpt(qr, 1, 2).\n\
             \x20               Inspect o2:\n\
             \x20                   When Ok (ov):\n\
             \x20                       Inspect ov:\n\
             \x20                           When Some (v):\n\
             \x20                               Show \"some <{{v}}>\".\n\
             \x20                           When None:\n\
             \x20                               Show \"none\".\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let o3 be pgTextAtOpt(qr, 1, 3).\n\
             \x20               Inspect o3:\n\
             \x20                   When Ok (ov):\n\
             \x20                       Inspect ov:\n\
             \x20                           When Some (v):\n\
             \x20                               Show \"some <{{v}}>\".\n\
             \x20                           When None:\n\
             \x20                               Show \"none\".\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let leg be pgTextAt(qr, 1, 2).\n\
             \x20               Inspect leg:\n\
             \x20                   When Ok (v):\n\
             \x20                       Show \"<{{v}}>\".\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let closed be pgClose(pgc).\n\
             \x20       Show \"done\".\n\
             \x20   When Err (e):\n\
             \x20       Show e.\n"
        )
    };
    let mut exec_reply = empty_tagged(b'2');
    exec_reply.extend(row_description(&[("a", 23), ("b", 25), ("c", 25)]));
    exec_reply.extend(data_row(&[Some("42"), None, Some("")]));
    exec_reply.extend(command_complete("SELECT 1"));
    exec_reply.extend(ready_for_query());
    let mut steps = trust_preamble();
    steps.push(Step::Expect(exec_flight_bytes("bind_s1_null_second.hex")));
    steps.push(Step::Send(exec_reply));
    steps.push(Step::Expect(fixture_bytes("terminate.hex")));
    lock_both_tiers(
        program,
        &steps,
        &[
            "false",
            "true",
            "some <42>",
            "none",
            "some <>",
            "<>",
            "done",
        ],
    )
    .await;
}

/// An ErrorResponse mid-extended-flow (here: at Parse) drains to
/// ReadyForQuery and surfaces as the Err — and the connection stays usable
/// for a simple query afterwards. The server skips to Sync after an error,
/// exactly the real backend's discipline.
#[tokio::test]
async fn error_mid_extended_flow_recovers() {
    let program = |port: u16| {
        format!(
            "## Main\n\
             Let cR be pgConnect(\"postgres://alice:secret@127.0.0.1:{port}/appdb\").\n\
             Inspect cR:\n\
             \x20   When Ok (pgc):\n\
             \x20       Let ps: Seq of Text be [\"42\", \"hello\"].\n\
             \x20       Let qR be pgQueryParams(pgc, \"SELECT $1::int4, $2::text\", ps).\n\
             \x20       Inspect qR:\n\
             \x20           When Ok (qr):\n\
             \x20               Show \"unexpectedly ok\".\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let q2 be pgQuery(pgc, \"SELECT 1\").\n\
             \x20       Inspect q2:\n\
             \x20           When Ok (qr):\n\
             \x20               Let v be pgIntAt(qr, 1, 1).\n\
             \x20               Inspect v:\n\
             \x20                   When Ok (n):\n\
             \x20                       Show n.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let closed be pgClose(pgc).\n\
             \x20       Show \"done\".\n\
             \x20   When Err (e):\n\
             \x20       Show e.\n"
        )
    };
    let mut error_reply = error_syntax();
    error_reply.extend(ready_for_query());
    let mut select1_reply = row_description(&[("?column?", 23)]);
    select1_reply.extend(data_row(&[Some("1")]));
    select1_reply.extend(command_complete("SELECT 1"));
    select1_reply.extend(ready_for_query());
    let qp_flight =
        query_params_flight_bytes("SELECT $1::int4, $2::text", &[Some("42"), Some("hello")]);
    let mut steps = trust_preamble();
    steps.push(Step::Expect(qp_flight));
    steps.push(Step::Send(error_reply));
    steps.push(Step::Expect(query_frame("SELECT 1")));
    steps.push(Step::Send(select1_reply));
    steps.push(Step::Expect(fixture_bytes("terminate.hex")));
    lock_both_tiers(
        program,
        &steps,
        &[
            "ERROR 42601: syntax error at or near \"BOGUS\"",
            "1",
            "done",
        ],
    )
    .await;
}

/// The reference encoding of the whole pgQueryParams flight (unnamed Parse +
/// Bind + Describe('P') + Execute(0) + Sync), via the postgres-protocol
/// crate — the client must emit EXACTLY these bytes in one conversation.
fn query_params_flight_bytes(sql: &str, params: &[Option<&str>]) -> Vec<u8> {
    use postgres_protocol::message::frontend;
    use postgres_protocol::IsNull;
    let mut buf = bytes::BytesMut::new();
    frontend::parse("", sql, [], &mut buf).expect("parse encodes");
    frontend::bind(
        "",
        "",
        [],
        params.iter(),
        |v, buf| match v {
            None => Ok::<_, Box<dyn std::error::Error + Sync + Send>>(IsNull::Yes),
            Some(s) => {
                buf.extend_from_slice(s.as_bytes());
                Ok(IsNull::No)
            }
        },
        [0i16],
        &mut buf,
    )
    .unwrap_or_else(|_| panic!("the reference bind encoder failed"));
    frontend::describe(b'P', "", &mut buf).expect("describe encodes");
    frontend::execute("", 0, &mut buf).expect("execute encodes");
    frontend::sync(&mut buf);
    buf.to_vec()
}

/// The unnamed fast path end-to-end: one flight, ParseComplete +
/// BindComplete + rows — including a NULL param round-tripping to a NULL
/// cell, all frames byte-exact against the reference encoder.
#[tokio::test]
async fn query_params_unnamed_fast_path() {
    let program = |port: u16| {
        format!(
            "## Main\n\
             Let cR be pgConnect(\"postgres://alice:secret@127.0.0.1:{port}/appdb\").\n\
             Inspect cR:\n\
             \x20   When Ok (pgc):\n\
             \x20       Let ps: Seq of Text be [\"7\", \"\"].\n\
             \x20       Let ns: Seq of Bool be [false, true].\n\
             \x20       Let qR be pgQueryParamsN(pgc, \"SELECT $1::int4, $2::text\", ps, ns).\n\
             \x20       Inspect qR:\n\
             \x20           When Ok (qr):\n\
             \x20               Let v1 be pgIntAt(qr, 1, 1).\n\
             \x20               Inspect v1:\n\
             \x20                   When Ok (v):\n\
             \x20                       Show v.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let o2 be pgTextAtOpt(qr, 1, 2).\n\
             \x20               Inspect o2:\n\
             \x20                   When Ok (ov):\n\
             \x20                       Inspect ov:\n\
             \x20                           When Some (v):\n\
             \x20                               Show \"some <{{v}}>\".\n\
             \x20                           When None:\n\
             \x20                               Show \"none\".\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let closed be pgClose(pgc).\n\
             \x20       Show \"done\".\n\
             \x20   When Err (e):\n\
             \x20       Show e.\n"
        )
    };
    let mut reply = empty_tagged(b'1');
    reply.extend(empty_tagged(b'2'));
    reply.extend(row_description(&[("int4", 23), ("text", 25)]));
    reply.extend(data_row(&[Some("7"), None]));
    reply.extend(command_complete("SELECT 1"));
    reply.extend(ready_for_query());
    let mut steps = trust_preamble();
    steps.push(Step::Expect(query_params_flight_bytes(
        "SELECT $1::int4, $2::text",
        &[Some("7"), None],
    )));
    steps.push(Step::Send(reply));
    steps.push(Step::Expect(fixture_bytes("terminate.hex")));
    lock_both_tiers(program, &steps, &["7", "none", "done"]).await;
}

/// TORN DELIVERY over the extended protocol: the entire reply stream arrives
/// ONE BYTE AT A TIME — framing must reassemble identically.
#[tokio::test]
async fn torn_extended_flow_reassembles() {
    let program = |port: u16| {
        format!(
            "## Main\n\
             Let cR be pgConnect(\"postgres://alice:secret@127.0.0.1:{port}/appdb\").\n\
             Inspect cR:\n\
             \x20   When Ok (pgc):\n\
             \x20       Let ps: Seq of Text be [\"7\", \"\"].\n\
             \x20       Let ns: Seq of Bool be [false, true].\n\
             \x20       Let qR be pgQueryParamsN(pgc, \"SELECT $1::int4, $2::text\", ps, ns).\n\
             \x20       Inspect qR:\n\
             \x20           When Ok (qr):\n\
             \x20               Let v1 be pgIntAt(qr, 1, 1).\n\
             \x20               Inspect v1:\n\
             \x20                   When Ok (v):\n\
             \x20                       Show v.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let o2 be pgTextAtOpt(qr, 1, 2).\n\
             \x20               Inspect o2:\n\
             \x20                   When Ok (ov):\n\
             \x20                       Inspect ov:\n\
             \x20                           When Some (v):\n\
             \x20                               Show \"some <{{v}}>\".\n\
             \x20                           When None:\n\
             \x20                               Show \"none\".\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let closed be pgClose(pgc).\n\
             \x20       Show \"done\".\n\
             \x20   When Err (e):\n\
             \x20       Show e.\n"
        )
    };
    let mut reply = empty_tagged(b'1');
    reply.extend(empty_tagged(b'2'));
    reply.extend(row_description(&[("int4", 23), ("text", 25)]));
    reply.extend(data_row(&[Some("7"), None]));
    reply.extend(command_complete("SELECT 1"));
    reply.extend(ready_for_query());
    let mut steps = trust_preamble();
    steps.push(Step::Expect(query_params_flight_bytes(
        "SELECT $1::int4, $2::text",
        &[Some("7"), None],
    )));
    steps.push(Step::SendSlow(reply));
    steps.push(Step::Expect(fixture_bytes("terminate.hex")));
    lock_both_tiers(program, &steps, &["7", "none", "done"]).await;
}

/// A server that (against our max_rows 0) sends PortalSuspended anyway is
/// drained GRACEFULLY: rows keep, no trap, the drain still ends at
/// ReadyForQuery.
#[tokio::test]
async fn portal_suspended_drains_gracefully() {
    let program = |port: u16| {
        format!(
            "## Main\n\
             Let cR be pgConnect(\"postgres://alice:secret@127.0.0.1:{port}/appdb\").\n\
             Inspect cR:\n\
             \x20   When Ok (pgc):\n\
             \x20       Let ps: Seq of Text be [\"7\", \"x\"].\n\
             \x20       Let qR be pgQueryParams(pgc, \"SELECT $1::int4, $2::text\", ps).\n\
             \x20       Inspect qR:\n\
             \x20           When Ok (qr):\n\
             \x20               Let rws be qr's rows.\n\
             \x20               Show length of rws.\n\
             \x20               Let tagT be qr's cmdTag.\n\
             \x20               Show \"<{{tagT}}>\".\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let closed be pgClose(pgc).\n\
             \x20       Show \"done\".\n\
             \x20   When Err (e):\n\
             \x20       Show e.\n"
        )
    };
    let mut reply = empty_tagged(b'1');
    reply.extend(empty_tagged(b'2'));
    reply.extend(row_description(&[("int4", 23), ("text", 25)]));
    reply.extend(data_row(&[Some("7"), Some("x")]));
    reply.extend(empty_tagged(b's'));
    reply.extend(ready_for_query());
    let mut steps = trust_preamble();
    steps.push(Step::Expect(query_params_flight_bytes(
        "SELECT $1::int4, $2::text",
        &[Some("7"), Some("x")],
    )));
    steps.push(Step::Send(reply));
    steps.push(Step::Expect(fixture_bytes("terminate.hex")));
    lock_both_tiers(program, &steps, &["1", "<>", "done"]).await;
}

/// The nulls-mask length contract: a mask that does not pair one entry per
/// param is an Err BEFORE any bytes go out — the server sees nothing but
/// the Terminate.
#[tokio::test]
async fn mismatched_nulls_mask_is_an_err_before_send() {
    let program = |port: u16| {
        format!(
            "## Main\n\
             Let cR be pgConnect(\"postgres://alice:secret@127.0.0.1:{port}/appdb\").\n\
             Inspect cR:\n\
             \x20   When Ok (pgc):\n\
             \x20       Let ps: Seq of Text be [\"a\", \"b\"].\n\
             \x20       Let ns: Seq of Bool be [true].\n\
             \x20       Let qR be pgExecPreparedN(pgc, \"s1\", ps, ns).\n\
             \x20       Inspect qR:\n\
             \x20           When Ok (qr):\n\
             \x20               Show \"unexpectedly ok\".\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let closed be pgClose(pgc).\n\
             \x20       Show \"done\".\n\
             \x20   When Err (e):\n\
             \x20       Show e.\n"
        )
    };
    let mut steps = trust_preamble();
    steps.push(Step::Expect(fixture_bytes("terminate.hex")));
    lock_both_tiers(
        program,
        &steps,
        &["pg: the nulls mask must have one entry per param", "done"],
    )
    .await;
}

/// A postmaster serving two connections at once, as a real one does: the session runs `steps1`
/// while the cancel connection, accepted beside it, runs `steps2` and is then closed — the close is
/// how a postmaster acknowledges a cancel. Returns the session's Expect pairs, then the cancel
/// connection's.
fn spawn_two_conn_server(
    steps1: Vec<Step>,
    steps2: Vec<Step>,
) -> (u16, tokio::task::JoinHandle<Vec<(Vec<u8>, Vec<u8>)>>) {
    async fn run_script(
        s: &mut tokio::net::TcpStream,
        steps: Vec<Step>,
    ) -> Vec<(Vec<u8>, Vec<u8>)> {
        let mut reads = Vec::new();
        for step in steps {
            match step {
                Step::Expect(want) => expect_bytes(s, want, &mut reads).await,
                Step::Send(bytes) => {
                    s.write_all(&bytes).await.expect("server write");
                    s.flush().await.ok();
                }
                Step::SendSlow(bytes) => {
                    for b in bytes {
                        s.write_all(&[b]).await.expect("server slow write");
                        s.flush().await.ok();
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                }
                Step::Close => {
                    s.shutdown().await.ok();
                    break;
                }
            }
        }
        reads
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("binds");
    listener.set_nonblocking(true).expect("nonblocking");
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        let listener = TcpListener::from_std(listener).expect("tokio listener");
        let (mut session, _) = listener.accept().await.expect("client connects");
        let session = tokio::spawn(async move {
            let reads = run_script(&mut session, steps1).await;
            (session, reads)
        });
        let (mut cancel, _) = listener.accept().await.expect("the cancel connection arrives");
        let cancel_reads = run_script(&mut cancel, steps2).await;
        cancel.shutdown().await.ok();
        let (session, mut reads) = session.await.expect("the session script runs");
        reads.extend(cancel_reads);
        tokio::time::sleep(Duration::from_millis(200)).await;
        drop(session);
        reads
    });
    (port, handle)
}

/// THE CANCEL BYTE LOCK: pgConnect captures BackendKeyData (pid 18, secret
/// 52 — the scripted values), and pgCancelRequest opens a SECOND connection
/// carrying EXACTLY the 16 fixture bytes. Both tiers.
#[tokio::test]
async fn cancel_request_sends_the_exact_packet_on_a_fresh_connection() {
    let program = |port: u16| {
        format!(
            "## Main\n\
             Let cR be pgConnect(\"postgres://alice:secret@127.0.0.1:{port}/appdb\").\n\
             Inspect cR:\n\
             \x20   When Ok (pgc):\n\
             \x20       Show pgc's backendPid.\n\
             \x20       Show pgc's backendSecret.\n\
             \x20       Let can be pgCancelRequest(pgc).\n\
             \x20       Inspect can:\n\
             \x20           When Ok (u):\n\
             \x20               Show \"cancel sent\".\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let closed be pgClose(pgc).\n\
             \x20       Show \"done\".\n\
             \x20   When Err (e):\n\
             \x20       Show e.\n"
        )
    };
    let expected = ["18", "52", "cancel sent", "done"];

    let mut all_lines = Vec::new();
    for use_vm in [false, true] {
        let steps1 = {
            let mut s = trust_preamble();
            s.push(Step::Expect(fixture_bytes("terminate.hex")));
            s
        };
        let steps2 = vec![Step::Expect(fixture_bytes(
            "cancel_request_pid18_secret52.hex",
        ))];
        let (port, server) = spawn_two_conn_server(steps1, steps2);
        let result = run_tier(&program(port), use_vm).await;
        assert!(
            result.error.is_none(),
            "tier vm={use_vm} errored: {:?}",
            result.error
        );
        let reads = tokio::time::timeout(Duration::from_secs(30), server)
            .await
            .expect("the two-connection server must finish")
            .expect("the two-connection server must not panic");
        for (i, (want, got)) in reads.iter().enumerate() {
            assert_eq!(
                hex(got),
                hex(want),
                "tier vm={use_vm}: frame {} diverged (the last is the 16-byte CancelRequest)",
                i + 1
            );
        }
        all_lines.push(result.lines);
    }
    assert_eq!(
        all_lines[0], all_lines[1],
        "TIER DIVERGENCE on the cancel path"
    );
    assert_eq!(all_lines[0], expected, "the cancel conversation drifted");
}

/// A ONE-connection scripted server that, after the script, asserts NO second
/// connection is ever offered — a stray cancel that opened a fresh socket would
/// be caught here. Returns the first connection's Expect pairs plus a flag: true
/// iff a SECOND connection arrived (the cancel-race smoking gun).
fn spawn_one_conn_reject_second(
    steps1: Vec<Step>,
) -> (
    u16,
    tokio::task::JoinHandle<(Vec<(Vec<u8>, Vec<u8>)>, bool)>,
) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("binds");
    listener.set_nonblocking(true).expect("nonblocking");
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        let listener = TcpListener::from_std(listener).expect("tokio listener");
        let (mut s, _) = listener.accept().await.expect("client connects");
        let mut reads = Vec::new();
        for step in steps1 {
            match step {
                Step::Expect(want) => {
                    let mut buf = vec![0u8; want.len()];
                    s.read_exact(&mut buf)
                        .await
                        .expect("the expected client bytes arrive");
                    reads.push((want, buf));
                }
                Step::Send(bytes) => {
                    s.write_all(&bytes).await.expect("server write");
                    s.flush().await.ok();
                }
                Step::SendSlow(bytes) => {
                    for b in bytes {
                        s.write_all(&[b]).await.expect("server slow write");
                        s.flush().await.ok();
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                }
                Step::Close => {
                    s.shutdown().await.ok();
                    break;
                }
            }
        }
        // A stray cancel would open a SECOND connection: give it a generous
        // window to arrive. If one does, that is the race — flag it.
        let second = tokio::time::timeout(Duration::from_millis(400), listener.accept())
            .await
            .is_ok();
        (reads, second)
    });
    (port, handle)
}

/// THE CANCEL-CORRELATION LOCK (defect G3-1): a best-effort cancel must be a
/// NO-OP when the target query already completed — otherwise the CancelRequest,
/// dispatched on a fresh connection to the postmaster, races the NEXT query and
/// can cancel IT (docs: "Canceling Requests in Progress" routes by backend pid,
/// which is reused for the session's next statement). `pgCancelInFlight(pgc,
/// false)` (no query in flight) must open NO second connection and leave the
/// following query untouched; the byte-exact fire path stays proven by
/// `cancel_request_sends_the_exact_packet_on_a_fresh_connection`.
#[tokio::test]
async fn cancel_after_completion_is_a_noop_and_never_disturbs_the_next_query() {
    let program = |port: u16| {
        format!(
            "## Main\n\
             Let cR be pgConnect(\"postgres://alice:secret@127.0.0.1:{port}/appdb\").\n\
             Inspect cR:\n\
             \x20   When Ok (pgc):\n\
             \x20       Let q1 be pgQuery(pgc, \"SELECT id, name FROM t\").\n\
             \x20       Inspect q1:\n\
             \x20           When Ok (qr):\n\
             \x20               Show \"q1 ok\".\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let can be pgCancelInFlight(pgc, false).\n\
             \x20       Inspect can:\n\
             \x20           When Ok (u):\n\
             \x20               Show \"cancel skipped\".\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let q2 be pgQuery(pgc, \"SELECT id, name FROM t\").\n\
             \x20       Inspect q2:\n\
             \x20           When Ok (qr):\n\
             \x20               Let v be pgTextAt(qr, 1, 2).\n\
             \x20               Inspect v:\n\
             \x20                   When Ok (t):\n\
             \x20                       Show t.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let closed be pgClose(pgc).\n\
             \x20       Show \"done\".\n\
             \x20   When Err (e):\n\
             \x20       Show e.\n"
        )
    };
    let expected = ["q1 ok", "cancel skipped", "alice", "done"];

    let mut all_lines = Vec::new();
    for use_vm in [false, true] {
        let steps1 = {
            let mut s = trust_preamble();
            s.push(Step::Expect(query_frame("SELECT id, name FROM t")));
            s.push(Step::Send(happy_reply()));
            s.push(Step::Expect(query_frame("SELECT id, name FROM t")));
            s.push(Step::Send(happy_reply()));
            s.push(Step::Expect(fixture_bytes("terminate.hex")));
            s
        };
        let (port, server) = spawn_one_conn_reject_second(steps1);
        let result = run_tier(&program(port), use_vm).await;
        assert!(
            result.error.is_none(),
            "tier vm={use_vm} errored: {:?}",
            result.error
        );
        let (reads, second) = tokio::time::timeout(Duration::from_secs(30), server)
            .await
            .expect("the one-connection server must finish")
            .expect("the one-connection server must not panic");
        for (i, (want, got)) in reads.iter().enumerate() {
            assert_eq!(
                hex(got),
                hex(want),
                "tier vm={use_vm}: frame {} diverged",
                i + 1
            );
        }
        assert!(
            !second,
            "tier vm={use_vm}: a cancel with no query in flight OPENED A SECOND CONNECTION — \
             that stray CancelRequest can cancel the following query (the G3-1 race)"
        );
        all_lines.push(result.lines);
    }
    assert_eq!(
        all_lines[0], all_lines[1],
        "TIER DIVERGENCE on the cancel-correlation path"
    );
    assert_eq!(
        all_lines[0], expected,
        "the cancel-correlation conversation drifted"
    );
}

/// Read exactly `want.len()` client bytes from `s` and record them beside what was wanted.
async fn expect_bytes(
    s: &mut tokio::net::TcpStream,
    want: Vec<u8>,
    reads: &mut Vec<(Vec<u8>, Vec<u8>)>,
) {
    let mut buf = vec![0u8; want.len()];
    s.read_exact(&mut buf)
        .await
        .expect("the expected client bytes arrive");
    reads.push((want, buf));
}

/// A postmaster that acts on a cancel as a real one does: it reads the 16-byte CancelRequest, signals
/// the backend, and only then closes the cancel connection — `processing` later here, so the window
/// is wide. The session's next `SELECT 41 + 1` is the statement the signal lands on when it reaches
/// the backend before the cancel connection closes (ErrorResponse 57014), and runs when it arrives
/// after (`42`): a backend waiting for its next command ignores a cancel. Returns every client frame
/// read on either connection, in order.
fn spawn_cancel_race_server(
    processing: Duration,
) -> (u16, tokio::task::JoinHandle<Vec<(Vec<u8>, Vec<u8>)>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("binds");
    listener.set_nonblocking(true).expect("nonblocking");
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        let listener = TcpListener::from_std(listener).expect("tokio listener");
        let mut reads = Vec::new();
        let (mut session, _) = listener.accept().await.expect("client connects");
        for step in trust_preamble() {
            match step {
                Step::Expect(want) => expect_bytes(&mut session, want, &mut reads).await,
                Step::Send(bytes) => session.write_all(&bytes).await.expect("server write"),
                Step::SendSlow(_) | Step::Close => unreachable!("the trust preamble expects and sends"),
            }
        }
        let (mut cancel, _) = listener.accept().await.expect("the cancel connection arrives");
        expect_bytes(
            &mut cancel,
            fixture_bytes("cancel_request_pid18_secret52.hex"),
            &mut reads,
        )
        .await;
        let signalled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let postmaster = {
            let signalled = signalled.clone();
            tokio::spawn(async move {
                tokio::time::sleep(processing).await;
                signalled.store(true, std::sync::atomic::Ordering::SeqCst);
                cancel.shutdown().await.ok();
            })
        };
        expect_bytes(&mut session, query_frame("SELECT 41 + 1"), &mut reads).await;
        let mut reply = if signalled.load(std::sync::atomic::Ordering::SeqCst) {
            let mut r = row_description(&[("?column?", 23)]);
            r.extend(data_row(&[Some("42")]));
            r.extend(command_complete("SELECT 1"));
            r
        } else {
            fields_message(
                b'E',
                &[
                    (b'S', "ERROR"),
                    (b'V', "ERROR"),
                    (b'C', "57014"),
                    (b'M', "canceling statement due to user request"),
                ],
            )
        };
        reply.extend(ready_for_query());
        session.write_all(&reply).await.expect("server write");
        expect_bytes(&mut session, fixture_bytes("terminate.hex"), &mut reads).await;
        postmaster.await.expect("the postmaster closes the cancel connection");
        tokio::time::sleep(Duration::from_millis(200)).await;
        reads
    });
    (port, handle)
}

/// THE CANCEL-ACKNOWLEDGEMENT LOCK: `pgCancelRequest` returns only once the postmaster has closed the
/// cancel connection — its acknowledgement that the backend was signalled — as libpq's PQcancel does
/// (fe-connect.c, internal_cancel: "Without this delay, we might issue another command only to find
/// that our cancel zaps that command instead of the one we thought we were canceling"). The signal then
/// reaches a backend waiting for its next command, which ignores it, so the statement sent after the
/// cancel runs: the postmaster here takes 300 ms, and the query that follows the cancel still answers
/// 42. Both tiers.
#[tokio::test]
async fn a_cancel_waits_for_the_postmaster_so_it_never_cancels_the_next_query() {
    let program = |port: u16| {
        format!(
            "## Main\n\
             Let cR be pgConnect(\"postgres://alice:secret@127.0.0.1:{port}/appdb\").\n\
             Inspect cR:\n\
             \x20   When Ok (pgc):\n\
             \x20       Let can be pgCancelRequest(pgc).\n\
             \x20       Inspect can:\n\
             \x20           When Ok (u):\n\
             \x20               Show \"cancel sent\".\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let q be pgQuery(pgc, \"SELECT 41 + 1\").\n\
             \x20       Inspect q:\n\
             \x20           When Ok (qr):\n\
             \x20               Let v be pgIntAt(qr, 1, 1).\n\
             \x20               Inspect v:\n\
             \x20                   When Ok (n):\n\
             \x20                       Show n.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let closed be pgClose(pgc).\n\
             \x20       Show \"done\".\n\
             \x20   When Err (e):\n\
             \x20       Show e.\n"
        )
    };
    let expected = ["cancel sent", "42", "done"];

    let mut all_lines = Vec::new();
    for use_vm in [false, true] {
        let (port, server) = spawn_cancel_race_server(Duration::from_millis(300));
        let result = run_tier(&program(port), use_vm).await;
        assert!(
            result.error.is_none(),
            "tier vm={use_vm} errored: {:?}",
            result.error
        );
        let reads = tokio::time::timeout(Duration::from_secs(30), server)
            .await
            .expect("the cancel-race server must finish")
            .expect("the cancel-race server must not panic");
        for (i, (want, got)) in reads.iter().enumerate() {
            assert_eq!(
                hex(got),
                hex(want),
                "tier vm={use_vm}: frame {} diverged",
                i + 1
            );
        }
        all_lines.push(result.lines);
    }
    assert_eq!(
        all_lines[0], all_lines[1],
        "TIER DIVERGENCE on the cancel-acknowledgement path"
    );
    assert_eq!(
        all_lines[0], expected,
        "a cancel returned before the postmaster processed it, and it cancelled the next query"
    );
}

/// Read one backend-shaped frame the CLIENT sent: tag byte + Int32 length +
/// body. Returns (tag, body). Used by the SCRAM fault-injection server to pick
/// the client-first (SASLInitialResponse) nonce out of the wire.
async fn read_client_frame<S: tokio::io::AsyncRead + Unpin>(s: &mut S) -> (u8, Vec<u8>) {
    use tokio::io::AsyncReadExt;
    let mut tag = [0u8; 1];
    s.read_exact(&mut tag).await.expect("client frame tag");
    let mut len = [0u8; 4];
    s.read_exact(&mut len).await.expect("client frame length");
    let n = i32::from_be_bytes(len) as usize - 4;
    let mut body = vec![0u8; n];
    if n > 0 {
        s.read_exact(&mut body).await.expect("client frame body");
    }
    (tag[0], body)
}

/// What the SCRAM fault-injection server does AFTER it has sent the
/// SASLContinue (server-first) and the client has replied with the
/// client-final: either go silent forever, or half-close the socket. Both are
/// the shapes a real postmaster leaves behind when it hits its
/// `authentication_timeout` (postmaster.c `AuthenticationTimeout`, default 60s)
/// on a failed SCRAM — a bounded client must not block on either.
#[derive(Clone, Copy)]
enum ScramFault {
    /// Never send the server-final and never close — a truly silent peer.
    GoSilent,
    /// Shut the socket down mid-handshake (FIN) right after server-first.
    CloseAfterServerFirst,
}

/// A SCRAM server that authenticates far enough to reach the SASL exchange,
/// then injects a fault. It reads the client-first nonce and echoes an extended
/// server-nonce so the client proceeds to the client-final (proving the client
/// is genuinely mid-handshake, not bailing on a malformed frame).
fn spawn_scram_fault_server(fault: ScramFault) -> (u16, tokio::task::JoinHandle<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("binds");
    listener.set_nonblocking(true).expect("nonblocking");
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        let listener = TcpListener::from_std(listener).expect("tokio listener");
        let (mut s, _) = listener.accept().await.expect("client connects");

        // Startup: read the untagged StartupMessage (Int32 len + body).
        {
            let mut len = [0u8; 4];
            s.read_exact(&mut len).await.expect("startup length");
            let n = i32::from_be_bytes(len) as usize - 4;
            let mut body = vec![0u8; n];
            s.read_exact(&mut body).await.expect("startup body");
        }

        // AuthenticationSASL (code 10): offer SCRAM-SHA-256 as a cstring list + NUL.
        {
            let mut body = 10i32.to_be_bytes().to_vec();
            body.extend_from_slice(&cstr("SCRAM-SHA-256"));
            body.push(0);
            s.write_all(&tagged(b'R', &body))
                .await
                .expect("send SASL offer");
            s.flush().await.ok();
        }

        // Read the client-first (SASLInitialResponse, tag 'p'): mech cstring +
        // Int32 len + the client-first bytes "n,,n=,r=<clientNonce>".
        let (_tag, first_body) = read_client_frame(&mut s).await;
        let text = String::from_utf8_lossy(&first_body);
        let client_nonce = text
            .rsplit("r=")
            .next()
            .unwrap_or("")
            .trim_end_matches(char::from(0))
            .to_string();

        // SASLContinue (code 11) server-first: r=<clientNonce+serverExt>,
        // s=<base64 salt>,i=4096 — a syntactically valid server-first so the
        // client computes its proof and sends the client-final.
        {
            let server_first = format!("r={client_nonce}deadbeef,s=cid2b2VQ,i=4096");
            let mut body = 11i32.to_be_bytes().to_vec();
            body.extend_from_slice(server_first.as_bytes());
            s.write_all(&tagged(b'R', &body))
                .await
                .expect("send server-first");
            s.flush().await.ok();
        }

        match fault {
            ScramFault::CloseAfterServerFirst => {
                s.shutdown().await.ok();
            }
            ScramFault::GoSilent => {
                // Drain the client-final so the client is genuinely blocked on
                // the NEXT recv, then go silent forever (until the test drops us).
                let _ = read_client_frame(&mut s).await;
                std::future::pending::<()>().await;
            }
        }
    });
    (port, handle)
}

/// THE SCRAM-FAIL BOUNDEDNESS LOCK (defect G3-2): a failed SCRAM handshake must
/// surface a BOUNDED `Err` and never leave the client blocked on a dead socket
/// until PostgreSQL's `authentication_timeout` (default 60s). `pgConnect` sets a
/// connect/auth DEADLINE via `tcpSetTimeout` right after connect, so a silent
/// or half-closed peer during the handshake yields an Err PROMPTLY. We assert
/// the whole connect completes far inside that 60s server timeout (a generous
/// bound that still fails loudly if the deadline is missing and the client
/// hangs) on BOTH tiers, for BOTH silence and a mid-handshake close.
#[tokio::test]
async fn a_failed_scram_handshake_surfaces_a_bounded_err_and_never_hangs() {
    let program = |port: u16| {
        format!(
            "## Main\n\
             Let cR be pgConnect(\"postgres://alice:secret@127.0.0.1:{port}/appdb\").\n\
             Inspect cR:\n\
             \x20   When Ok (pgc):\n\
             \x20       Show \"unexpectedly connected\".\n\
             \x20   When Err (e):\n\
             \x20       Show \"refused\".\n"
        )
    };

    for fault in [ScramFault::GoSilent, ScramFault::CloseAfterServerFirst] {
        for use_vm in [false, true] {
            let (port, server) = spawn_scram_fault_server(fault);
            let started = std::time::Instant::now();
            // A hard 20s bound: comfortably below PostgreSQL's 60s auth timeout,
            // yet far above the client's own auth deadline — so a MISSING deadline
            // (the hang) trips this, while a present one passes with wide margin.
            let result =
                tokio::time::timeout(Duration::from_secs(20), run_tier(&program(port), use_vm))
                    .await
                    .expect(
                        "a failed SCRAM handshake must not hang the client past its auth deadline",
                    );
            let elapsed = started.elapsed();
            server.abort();
            assert!(
                result.error.is_none(),
                "tier vm={use_vm}: a failed SCRAM must be a value, not a trap: {:?}",
                result.error
            );
            assert_eq!(
                result.lines,
                vec!["refused"],
                "tier vm={use_vm}: a failed SCRAM handshake must refuse cleanly"
            );
            assert!(
                elapsed < Duration::from_secs(15),
                "tier vm={use_vm}: pgConnect took {elapsed:?} on a dead SCRAM socket — \
                 the connect/auth deadline is missing (the G3-2 hang)"
            );
        }
    }
}

/// pgBeginIsolation validates its level against the closed set BEFORE
/// composing SQL — a bogus level never reaches the wire; the three legal
/// levels compose exactly the expected BEGIN statements (Expect'd
/// byte-for-byte as simple-protocol frames).
#[tokio::test]
async fn begin_isolation_validates_and_composes() {
    let program = |port: u16| {
        format!(
            "## Main\n\
             Let cR be pgConnect(\"postgres://alice:secret@127.0.0.1:{port}/appdb\").\n\
             Inspect cR:\n\
             \x20   When Ok (pgc):\n\
             \x20       Let bogus be pgBeginIsolation(pgc, \"chaotic evil\").\n\
             \x20       Inspect bogus:\n\
             \x20           When Ok (n):\n\
             \x20               Show \"unexpectedly ok\".\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let b1 be pgBeginIsolation(pgc, \"serializable\").\n\
             \x20       Inspect b1:\n\
             \x20           When Ok (n):\n\
             \x20               Show n.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let r1 be pgRollback(pgc).\n\
             \x20       Inspect r1:\n\
             \x20           When Ok (n):\n\
             \x20               Show n.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let closed be pgClose(pgc).\n\
             \x20       Show \"done\".\n\
             \x20   When Err (e):\n\
             \x20       Show e.\n"
        )
    };
    let mut begin_reply = command_complete("BEGIN");
    begin_reply.extend(ready_for_query());
    let mut rollback_reply = command_complete("ROLLBACK");
    rollback_reply.extend(ready_for_query());
    let mut steps = trust_preamble();
    steps.push(Step::Expect(query_frame(
        "BEGIN ISOLATION LEVEL serializable",
    )));
    steps.push(Step::Send(begin_reply));
    steps.push(Step::Expect(query_frame("ROLLBACK")));
    steps.push(Step::Send(rollback_reply));
    steps.push(Step::Expect(fixture_bytes("terminate.hex")));
    lock_both_tiers(
        program,
        &steps,
        &[
            "pgBeginIsolation: the isolation level must be 'read committed', 'repeatable read', or 'serializable'",
            "0",
            "0",
            "done",
        ],
    )
    .await;
}

/// pgSetStatementTimeout composes only the validated integer; a negative
/// timeout never reaches the wire.
#[tokio::test]
async fn set_statement_timeout_validates_and_composes() {
    let program = |port: u16| {
        format!(
            "## Main\n\
             Let cR be pgConnect(\"postgres://alice:secret@127.0.0.1:{port}/appdb\").\n\
             Inspect cR:\n\
             \x20   When Ok (pgc):\n\
             \x20       Let bad be pgSetStatementTimeout(pgc, -5).\n\
             \x20       Inspect bad:\n\
             \x20           When Ok (n):\n\
             \x20               Show \"unexpectedly ok\".\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let good be pgSetStatementTimeout(pgc, 1500).\n\
             \x20       Inspect good:\n\
             \x20           When Ok (n):\n\
             \x20               Show n.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let closed be pgClose(pgc).\n\
             \x20       Show \"done\".\n\
             \x20   When Err (e):\n\
             \x20       Show e.\n"
        )
    };
    let mut set_reply = command_complete("SET");
    set_reply.extend(ready_for_query());
    let mut steps = trust_preamble();
    steps.push(Step::Expect(query_frame("SET statement_timeout = 1500")));
    steps.push(Step::Send(set_reply));
    steps.push(Step::Expect(fixture_bytes("terminate.hex")));
    lock_both_tiers(
        program,
        &steps,
        &[
            "pgSetStatementTimeout: the timeout must be at least 0 milliseconds",
            "0",
            "done",
        ],
    )
    .await;
}

// ---------------------------------------------------------------------------
// Interop: a REAL protocol implementation (the pgwire crate) hosts the server
// ---------------------------------------------------------------------------

mod pgwire_server {
    use std::fmt::Debug;
    use std::sync::Arc;

    use async_trait::async_trait;
    use futures::{stream, Sink, StreamExt};
    use pgwire::api::auth::noop::NoopStartupHandler;
    use pgwire::api::auth::StartupHandler;
    use pgwire::api::query::SimpleQueryHandler;
    use pgwire::api::results::{
        DataRowEncoder, FieldFormat, FieldInfo, QueryResponse, Response, Tag,
    };
    use pgwire::api::store::PortalStore;
    use pgwire::api::{ClientInfo, ClientPortalStore, PgWireServerHandlers, Type};
    use pgwire::error::{PgWireError, PgWireResult};
    use pgwire::messages::{PgWireBackendMessage, PgWireFrontendMessage};

    pub struct CannedTable;

    #[async_trait]
    impl NoopStartupHandler for CannedTable {
        async fn post_startup<C>(
            &self,
            _client: &mut C,
            _message: PgWireFrontendMessage,
        ) -> PgWireResult<()>
        where
            C: ClientInfo + Sink<PgWireBackendMessage> + Unpin + Send,
            C::Error: Debug,
            PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
        {
            Ok(())
        }
    }

    #[async_trait]
    impl SimpleQueryHandler for CannedTable {
        async fn do_query<C>(&self, _client: &mut C, query: &str) -> PgWireResult<Vec<Response>>
        where
            C: ClientInfo + ClientPortalStore + Unpin + Send + Sync,
            C::PortalStore: PortalStore,
        {
            if query.starts_with("SELECT") {
                let f1 = FieldInfo::new("id".into(), None, None, Type::INT4, FieldFormat::Text);
                let f2 =
                    FieldInfo::new("name".into(), None, None, Type::VARCHAR, FieldFormat::Text);
                let schema = Arc::new(vec![f1, f2]);
                let data: Vec<(Option<i32>, Option<&str>)> = vec![
                    (Some(1), Some("alice")),
                    (Some(2), Some("bob")),
                    (Some(3), None),
                ];
                let mut encoder = DataRowEncoder::new(schema.clone());
                let rows = stream::iter(data).map(move |r| {
                    encoder.encode_field(&r.0)?;
                    encoder.encode_field(&r.1)?;
                    Ok(encoder.take_row())
                });
                Ok(vec![Response::Query(QueryResponse::new(schema, rows))])
            } else {
                Ok(vec![Response::Execution(
                    Tag::new("INSERT").with_oid(0).with_rows(3),
                )])
            }
        }
    }

    pub struct Factory {
        pub handler: Arc<CannedTable>,
    }

    impl PgWireServerHandlers for Factory {
        fn simple_query_handler(&self) -> Arc<impl SimpleQueryHandler> {
            self.handler.clone()
        }

        fn startup_handler(&self) -> Arc<impl StartupHandler> {
            self.handler.clone()
        }
    }

    // --- SCRAM-SHA-256 variant: pgwire's real server-side SASL verifier ---

    use pgwire::api::auth::sasl::scram::{gen_salted_password, ScramAuth};
    use pgwire::api::auth::sasl::SASLAuthStartupHandler;
    use pgwire::api::auth::{AuthSource, DefaultServerParameterProvider, LoginInfo, Password};

    /// PG16's scram_iterations is server-configurable, so a real server may
    /// advertise ANY count. 600 keeps the tree-walker's PBKDF2 inside the test
    /// budget AND proves the client honors the server-advertised i= (it would
    /// fail against a client that assumed the RFC's 4096). The 4096-iteration
    /// math itself is pinned by the RFC 7677 vectors in postgres_wire.rs.
    pub const SCRAM_ITERATIONS: usize = 600;

    #[derive(Debug)]
    pub struct SecretAuthDb;

    #[async_trait]
    impl AuthSource for SecretAuthDb {
        async fn get_password(&self, _login: &LoginInfo) -> PgWireResult<Password> {
            let salt = b"0123456789".to_vec();
            let hashed = gen_salted_password("secret", &salt, SCRAM_ITERATIONS);
            Ok(Password::new(Some(salt), hashed))
        }
    }

    pub struct ScramFactory {
        pub handler: Arc<CannedTable>,
    }

    impl PgWireServerHandlers for ScramFactory {
        fn simple_query_handler(&self) -> Arc<impl SimpleQueryHandler> {
            self.handler.clone()
        }

        fn startup_handler(&self) -> Arc<impl StartupHandler> {
            let mut scram = ScramAuth::new(Arc::new(SecretAuthDb));
            scram.set_iterations(SCRAM_ITERATIONS);
            let authenticator =
                SASLAuthStartupHandler::new(Arc::new(DefaultServerParameterProvider::default()))
                    .with_scram(scram);
            Arc::new(authenticator)
        }
    }
}

/// Our pure-LOGOS client connects to a server built on the pgwire crate — a
/// REAL, independent protocol implementation — and reads its canned table:
/// column names, type OIDs (INT4=23, VARCHAR=1043), rows including a NULL,
/// and an Execution tag ("INSERT 0 3" → 3). Both tiers agree.
#[tokio::test]
async fn pgwire_interop_simple_query_agrees_across_tiers() {
    use std::sync::Arc;

    let program = |port: u16| {
        format!(
            "## Main\n\
             Let cR be pgConnect(\"postgres://alice@127.0.0.1:{port}/appdb\").\n\
             Inspect cR:\n\
             \x20   When Ok (pgc):\n\
             \x20       Let qR be pgQuery(pgc, \"SELECT id, name FROM t\").\n\
             \x20       Inspect qR:\n\
             \x20           When Ok (qr):\n\
             \x20               Let colsSeq be qr's columns.\n\
             \x20               Let colsText be join(colsSeq, \",\").\n\
             \x20               Show colsText.\n\
             \x20               Show qr's typeOids.\n\
             \x20               Let rws be qr's rows.\n\
             \x20               Let nrows be length of rws.\n\
             \x20               Show nrows.\n\
             \x20               Let a1 be pgTextAt(qr, 1, 2).\n\
             \x20               Inspect a1:\n\
             \x20                   When Ok (v):\n\
             \x20                       Show v.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let a2 be pgIntAt(qr, 3, 1).\n\
             \x20               Inspect a2:\n\
             \x20                   When Ok (v):\n\
             \x20                       Show v.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let a3 be pgTextAt(qr, 3, 2).\n\
             \x20               Inspect a3:\n\
             \x20                   When Ok (v):\n\
             \x20                       Show \"<{{v}}>\".\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let eR be pgExecute(pgc, \"INSERT INTO t SELECT 9\").\n\
             \x20       Inspect eR:\n\
             \x20           When Ok (n):\n\
             \x20               Show n.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let closed be pgClose(pgc).\n\
             \x20       Show \"done\".\n\
             \x20   When Err (e):\n\
             \x20       Show e.\n"
        )
    };
    let expected = [
        "id,name",
        "[23, 1043]",
        "3",
        "alice",
        "3",
        "<>",
        "3",
        "done",
    ];

    let mut all_lines = Vec::new();
    for use_vm in [false, true] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("binds");
        listener.set_nonblocking(true).expect("nonblocking");
        let port = listener.local_addr().unwrap().port();
        let factory = Arc::new(pgwire_server::Factory {
            handler: Arc::new(pgwire_server::CannedTable),
        });
        let server = tokio::spawn(async move {
            let listener = TcpListener::from_std(listener).expect("tokio listener");
            let (socket, _) = listener.accept().await.expect("client connects");
            let _ = pgwire::tokio::process_socket(socket, None, factory).await;
        });
        let result = run_tier(&program(port), use_vm).await;
        assert!(
            result.error.is_none(),
            "tier vm={use_vm} errored against pgwire: {:?}",
            result.error
        );
        server.abort();
        all_lines.push(result.lines);
    }
    assert_eq!(
        all_lines[0], all_lines[1],
        "TIER DIVERGENCE against the pgwire server"
    );
    assert_eq!(all_lines[0], expected, "the pgwire interop output drifted");
}

/// FULL SCRAM-SHA-256 against pgwire's REAL server-side SASL verifier: our
/// pure-LOGOS client runs the whole RFC 5802 exchange — fresh random nonce,
/// client-first "n,,n=,r=…", PBKDF2 salted password, ClientProof, and the
/// ServerSignature check — and the independent implementation ACCEPTS the
/// proof, serves a query, and both tiers agree. A WRONG password must be
/// REJECTED with an Err (never a hang), proving the proof actually matters.
#[tokio::test]
async fn pgwire_interop_scram_sha256_full_exchange() {
    use std::sync::Arc;

    /// The SCRAM legs run PBKDF2 on the tree-walker (compute-heavy, not IO),
    /// so this test carries a wider hang guard than the scripted-frame locks.
    async fn run_tier_slow(
        program: &str,
        use_vm: bool,
    ) -> logicaffeine_compile::interpreter::InterpreterResult {
        let fut = async {
            if use_vm {
                run_vm_io_async(program).await
            } else {
                interpret_for_ui(program).await
            }
        };
        tokio::time::timeout(Duration::from_secs(180), fut)
            .await
            .expect("the SCRAM program must not hang")
    }

    let program = |port: u16, password: &str| {
        format!(
            "## Main\n\
             Let cR be pgConnect(\"postgres://alice:{password}@127.0.0.1:{port}/appdb\").\n\
             Inspect cR:\n\
             \x20   When Ok (pgc):\n\
             \x20       Let qR be pgQuery(pgc, \"SELECT id, name FROM t\").\n\
             \x20       Inspect qR:\n\
             \x20           When Ok (qr):\n\
             \x20               Let colsSeq be qr's columns.\n\
             \x20               Let colsText be join(colsSeq, \",\").\n\
             \x20               Show colsText.\n\
             \x20               Let rws be qr's rows.\n\
             \x20               Let nrows be length of rws.\n\
             \x20               Show nrows.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let closed be pgClose(pgc).\n\
             \x20       Show \"done\".\n\
             \x20   When Err (e):\n\
             \x20       Show \"refused: {{e}}\".\n"
        )
    };

    let spawn_scram_server = || {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("binds");
        listener.set_nonblocking(true).expect("nonblocking");
        let port = listener.local_addr().unwrap().port();
        let factory = Arc::new(pgwire_server::ScramFactory {
            handler: Arc::new(pgwire_server::CannedTable),
        });
        let server = tokio::spawn(async move {
            let listener = TcpListener::from_std(listener).expect("tokio listener");
            let (socket, _) = listener.accept().await.expect("client connects");
            let _ = pgwire::tokio::process_socket(socket, None, factory).await;
        });
        (port, server)
    };

    // The RIGHT password authenticates and reads the table, on both tiers.
    let mut all_lines = Vec::new();
    for use_vm in [false, true] {
        let (port, server) = spawn_scram_server();
        let result = run_tier_slow(&program(port, "secret"), use_vm).await;
        assert!(
            result.error.is_none(),
            "tier vm={use_vm} errored during SCRAM: {:?}",
            result.error
        );
        server.abort();
        all_lines.push(result.lines);
    }
    assert_eq!(all_lines[0], all_lines[1], "TIER DIVERGENCE during SCRAM");
    assert_eq!(
        all_lines[0],
        ["id,name", "3", "done"],
        "the SCRAM happy path drifted"
    );

    // The WRONG password is rejected as an Err value on both tiers.
    for use_vm in [false, true] {
        let (port, server) = spawn_scram_server();
        let result = run_tier_slow(&program(port, "wrong"), use_vm).await;
        assert!(
            result.error.is_none(),
            "tier vm={use_vm} trapped on a rejected SCRAM login: {:?}",
            result.error
        );
        server.abort();
        assert_eq!(
            result.lines.len(),
            1,
            "a rejected login is exactly one line"
        );
        assert!(
            result.lines[0].starts_with("refused: "),
            "tier vm={use_vm}: expected a refused-line, got {:?}",
            result.lines[0]
        );
    }
}

#[tokio::test]
async fn zzz_debug_scram_deadline_timing() {
    for fault in [ScramFault::GoSilent, ScramFault::CloseAfterServerFirst] {
        let (port, server) = spawn_scram_fault_server(fault);
        let program = format!(
            "## Main\n\
             Let cR be pgConnect(\"postgres://alice:secret@127.0.0.1:{port}/appdb\").\n\
             Inspect cR:\n\
             \x20   When Ok (pgc):\n\
             \x20       Show \"connected\".\n\
             \x20   When Err (e):\n\
             \x20       Show e.\n"
        );
        let t = std::time::Instant::now();
        let result = tokio::time::timeout(Duration::from_secs(200), interpret_for_ui(&program))
            .await
            .expect("no hang");
        eprintln!(
            "ZZZ elapsed={:?} lines={:?} err={:?}",
            t.elapsed(),
            result.lines,
            result.error
        );
        server.abort();
    }
}
