//! ════════════════════════════════════════════════════════════════════════════════════════════
//! A3 — POSTGRES WIRE GOLDEN LOCKS: the pure-LOGOS byte helpers of `assets/std/postgres.lg`
//! (the PostgreSQL v3 frontend/backend protocol, written IN LOGOS on the tcp.lg natives) are
//! pinned to exact hex, byte for byte, on the tree-walker AND the VM (`assert_all_tiers`).
//!
//! Three-way agreement: every message hex here is (1) hand-derived from the PostgreSQL
//! documentation "Message Formats" (protocol 3.0) and the postgres source itself
//! (src/include/libpq/protocol.h, fe-protocol3.c), (2) re-encoded by the `postgres-protocol`
//! crate in the Rust differential tests at the bottom, and (3) produced by our LOGOS
//! functions in the locks — all three must land on the SAME bytes.
//!
//!  ⚠️  YOU DO NOT GET TO WEAKEN THESE LOCKS TO MAKE THEM PASS.  ⚠️
//!  A mismatch means postgres.lg drifted off the wire protocol — fix the module, never the hex.
//! ════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::assert_all_tiers;

// ---------------------------------------------------------------------------
// Big-endian integer encoders — the atoms every v3 message is built from
// ---------------------------------------------------------------------------

/// The protocol version number 196608 (3 << 16, i.e. protocol 3.0) encodes as
/// exactly the four bytes 00 03 00 00 — the first payload word of every
/// StartupMessage a v3 frontend sends.
#[test]
fn pg_i32be_encodes_the_protocol_version() {
    assert_all_tiers(
        "## Main\n\
         Show bytesToHex(pgI32be(196608)).\n",
        "00030000",
    );
}

/// Int32 -1 is the two's-complement ff ff ff ff — the DataRow column length
/// PostgreSQL uses to mark NULL, so signed encoding must be exact.
#[test]
fn pg_i32be_encodes_negative_one_as_twos_complement() {
    assert_all_tiers(
        "## Main\n\
         Show bytesToHex(pgI32be(-1)).\n\
         Show bytesToHex(pgI32be(0)).\n\
         Show bytesToHex(pgI32be(2147483647)).\n\
         Show bytesToHex(pgI32be(-2147483648)).\n",
        "ffffffff\n00000000\n7fffffff\n80000000",
    );
}

/// Int16 big-endian: 1 → 00 01, -1 → ff ff (RowDescription/DataRow field counts
/// are Int16; -1 appears as the "no fixed size" typlen).
#[test]
fn pg_i16be_encodes_one_and_negative_one() {
    assert_all_tiers(
        "## Main\n\
         Show bytesToHex(pgI16be(1)).\n\
         Show bytesToHex(pgI16be(-1)).\n\
         Show bytesToHex(pgI16be(0)).\n\
         Show bytesToHex(pgI16be(32767)).\n\
         Show bytesToHex(pgI16be(-32768)).\n",
        "0001\nffff\n0000\n7fff\n8000",
    );
}

/// pgReadI32be inverts pgI32be at any 1-based offset, INCLUDING the signed
/// values (NULL's -1 and both int32 extremes must round-trip exactly).
#[test]
fn pg_read_i32be_round_trips_signed_values() {
    assert_all_tiers(
        "## Main\n\
         Show pgReadI32be(pgI32be(196608), 1).\n\
         Show pgReadI32be(pgI32be(-1), 1).\n\
         Show pgReadI32be(pgI32be(2147483647), 1).\n\
         Show pgReadI32be(pgI32be(-2147483648), 1).\n\
         Show pgReadI32be(pgI32be(0), 1).\n",
        "196608\n-1\n2147483647\n-2147483648\n0",
    );
}

/// pgReadI16be inverts pgI16be, signed, at any offset.
#[test]
fn pg_read_i16be_round_trips_signed_values() {
    assert_all_tiers(
        "## Main\n\
         Show pgReadI16be(pgI16be(1), 1).\n\
         Show pgReadI16be(pgI16be(-1), 1).\n\
         Show pgReadI16be(pgI16be(32767), 1).\n\
         Show pgReadI16be(pgI16be(-32768), 1).\n",
        "1\n-1\n32767\n-32768",
    );
}

/// Reads honor the 1-based offset: the same four version bytes surrounded by
/// noise still decode from the middle of a longer frame.
#[test]
fn pg_read_be_honors_the_offset() {
    assert_all_tiers(
        "## Main\n\
         Let frame be [255, 0, 3, 0, 0, 255].\n\
         Show pgReadI32be(frame, 2).\n\
         Let half be [9, 9, 255, 255].\n\
         Show pgReadI16be(half, 3).\n",
        "196608\n-1",
    );
}

// ---------------------------------------------------------------------------
// C strings — the protocol's null-terminated text atom
// ---------------------------------------------------------------------------

/// pgCString("user") is the five bytes 75 73 65 72 00 — UTF-8 text plus the
/// terminating zero, exactly as the docs' "String(s)" field type.
#[test]
fn pg_cstring_appends_the_terminating_nul() {
    assert_all_tiers(
        "## Main\n\
         Show bytesToHex(pgCString(\"user\")).\n\
         Show bytesToHex(pgCString(\"\")).\n",
        "7573657200\n00",
    );
}

// ---------------------------------------------------------------------------
// StartupMessage — length-prefixed, NO tag byte, protocol 3.0
// ---------------------------------------------------------------------------

/// The exact StartupMessage for (user "u", database "d"), derived by hand from
/// the docs: Int32 length 27 (includes itself), Int32 196608, then the
/// cstring pairs user\0u\0database\0d\0 and the trailing terminator 00. The
/// startup message is the ONE untagged frontend message in v3.
#[test]
fn pg_startup_message_user_u_database_d_is_byte_exact() {
    assert_all_tiers(
        "## Main\n\
         Show bytesToHex(pgStartupMessage(\"u\", \"d\")).\n",
        "0000001b0003000075736572007500646174616261736500640000",
    );
}

/// The startup bytes the scripted-server suite pins as a fixture: user "alice",
/// database "appdb". Locked here too so the fixture, this lock, and the
/// postgres-protocol differential all agree on the same 35 bytes.
#[test]
fn pg_startup_message_alice_appdb_is_byte_exact() {
    assert_all_tiers(
        "## Main\n\
         Show bytesToHex(pgStartupMessage(\"alice\", \"appdb\")).\n",
        "00000023000300007573657200616c6963650064617461626173650061707064620000",
    );
}

// ---------------------------------------------------------------------------
// Tagged frontend messages: Query 'Q', PasswordMessage 'p', Terminate 'X'
// ---------------------------------------------------------------------------

/// Query is tag 'Q' (0x51), Int32 length (self + body, excluding the tag),
/// then the SQL as a cstring: "SELECT 1" frames as 13 bytes after the tag.
#[test]
fn pg_query_message_select_1_is_byte_exact() {
    assert_all_tiers(
        "## Main\n\
         Show bytesToHex(pgQueryMessage(\"SELECT 1\")).\n",
        "510000000d53454c454354203100",
    );
}

/// PasswordMessage is tag 'p' (0x70) + length + password cstring — the
/// cleartext (AuthenticationCleartextPassword) response frame.
#[test]
fn pg_password_message_secret_is_byte_exact() {
    assert_all_tiers(
        "## Main\n\
         Show bytesToHex(pgPasswordMessage(\"secret\")).\n",
        "700000000b73656372657400",
    );
}

/// Terminate is tag 'X' (0x58) with an empty body: exactly 58 00 00 00 04.
#[test]
fn pg_terminate_message_is_byte_exact() {
    assert_all_tiers(
        "## Main\n\
         Show bytesToHex(pgTerminateMessage()).\n",
        "5800000004",
    );
}

/// SASLInitialResponse rides in a 'p' frame: mechanism cstring
/// "SCRAM-SHA-256", Int32 byte-length of the initial client response, then
/// those bytes verbatim. Locked with a fixed nonce so the frame is exact.
#[test]
fn pg_sasl_initial_message_is_byte_exact() {
    // body = "SCRAM-SHA-256\0" (14) + int32 25 + "n,,n=,r=fixednoncefixedno" (25)
    // len  = 4 + 14 + 4 + 25 = 47 = 0x2f
    assert_all_tiers(
        "## Main\n\
         Show bytesToHex(pgSaslInitialMessage(\"n,,n=,r=fixednoncefixedno\")).\n",
        "700000002f534352414d2d5348412d32353600000000196e2c2c6e3d2c723d66697865646e6f6e636566697865646e6f",
    );
}

/// SASLResponse is a 'p' frame whose body is the client-final message verbatim
/// (no mechanism, no inner length).
#[test]
fn pg_sasl_response_message_is_byte_exact() {
    // "c=biws,r=ab" = 11 bytes; len = 4 + 11 = 15 = 0x0f
    assert_all_tiers(
        "## Main\n\
         Show bytesToHex(pgSaslResponseMessage(\"c=biws,r=ab\")).\n",
        "700000000f633d626977732c723d6162",
    );
}

// ---------------------------------------------------------------------------
// Base64 (RFC 4648) — SCRAM's transport encoding, pure LOGOS over raw bytes
// ---------------------------------------------------------------------------

/// The seven RFC 4648 §10 test vectors, encoding. Each value is shown inside
/// angle brackets so the empty-string vector ("" → "") is genuinely observable
/// (a bare empty Show line would vanish under the harness trim).
#[test]
fn pg_base64_rfc4648_encode_vectors() {
    assert_all_tiers(
        "## Main\n\
         Let e0 be pgBase64(text_bytes(\"\")).\n\
         Show \"<{e0}>\".\n\
         Let e1 be pgBase64(text_bytes(\"f\")).\n\
         Show \"<{e1}>\".\n\
         Let e2 be pgBase64(text_bytes(\"fo\")).\n\
         Show \"<{e2}>\".\n\
         Let e3 be pgBase64(text_bytes(\"foo\")).\n\
         Show \"<{e3}>\".\n\
         Let e4 be pgBase64(text_bytes(\"foob\")).\n\
         Show \"<{e4}>\".\n\
         Let e5 be pgBase64(text_bytes(\"fooba\")).\n\
         Show \"<{e5}>\".\n\
         Let e6 be pgBase64(text_bytes(\"foobar\")).\n\
         Show \"<{e6}>\".\n",
        "<>\n<Zg==>\n<Zm8=>\n<Zm9v>\n<Zm9vYg==>\n<Zm9vYmE=>\n<Zm9vYmFy>",
    );
}

/// The same seven vectors, decoding — via the byte values so the empty case
/// and exact contents are visible.
#[test]
fn pg_base64_rfc4648_decode_vectors() {
    assert_all_tiers(
        "## Main\n\
         Show pgBase64Decode(\"\").\n\
         Show pgBase64Decode(\"Zg==\").\n\
         Show pgBase64Decode(\"Zm8=\").\n\
         Show pgBase64Decode(\"Zm9v\").\n\
         Show pgBase64Decode(\"Zm9vYg==\").\n\
         Show pgBase64Decode(\"Zm9vYmFy\").\n",
        "[]\n[102]\n[102, 111]\n[102, 111, 111]\n[102, 111, 111, 98]\n[102, 111, 111, 98, 97, 114]",
    );
}

/// Base64 is over BYTES, not text: 0x00, 0xff and friends must survive both
/// directions (SCRAM salts and HMAC proofs are arbitrary binary).
#[test]
fn pg_base64_handles_arbitrary_binary_bytes() {
    assert_all_tiers(
        "## Main\n\
         Show pgBase64([0, 255, 16]).\n\
         Show pgBase64Decode(\"AP8Q\").\n\
         Show pgBase64Decode(pgBase64([0, 1, 2, 253, 254, 255])).\n",
        "AP8Q\n[0, 255, 16]\n[0, 1, 2, 253, 254, 255]",
    );
}

/// The RFC 7677 salt decodes to the exact 16 bytes the SCRAM vectors need.
#[test]
fn pg_base64_decodes_the_rfc7677_salt() {
    assert_all_tiers(
        "## Main\n\
         Show pgBase64Decode(\"W22ZaJ0SNY7soEsUEjb6gQ==\").\n",
        "[91, 109, 153, 104, 157, 18, 53, 142, 236, 160, 75, 20, 18, 54, 250, 129]",
    );
}

// ---------------------------------------------------------------------------
// MD5 password (fe-auth.c pg_md5_encrypt): "md5" + md5(md5(password+user)+salt)
// ---------------------------------------------------------------------------

/// The libpq double-hash construction, pinned to an independently computed
/// vector (python3 hashlib): user "u", password "p", salt 01 02 03 04.
#[test]
fn pg_md5_password_matches_the_libpq_construction() {
    assert_all_tiers(
        "## Main\n\
         Show pgMd5Password(\"p\", \"u\", [1, 2, 3, 4]).\n",
        "md5facdc455923b0a58efea84a6d9ee0e76",
    );
}

/// The exact MD5 password the scripted-server fixture pins: user "alice",
/// password "secret", salt "qwer" (71 77 65 72).
#[test]
fn pg_md5_password_matches_the_fixture_vector() {
    assert_all_tiers(
        "## Main\n\
         Show pgMd5Password(\"secret\", \"alice\", [113, 119, 101, 114]).\n",
        "md521084184b0b97a0f5996f86647cee7aa",
    );
}

// ---------------------------------------------------------------------------
// SCRAM-SHA-256 — RFC 5802 / RFC 7677 worked example ("user" / "pencil")
// ---------------------------------------------------------------------------

const RFC7677_AUTH_MESSAGE: &str = "n=user,r=rOprNGfwEbeRWgbNEkqO,\
r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,s=W22ZaJ0SNY7soEsUEjb6gQ==,i=4096,\
c=biws,r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0";

/// ClientProof = ClientKey XOR HMAC(StoredKey, AuthMessage), with
/// SaltedPassword = Hi("pencil", salt, 4096) — must reproduce the RFC 7677
/// worked example's p= value bit for bit.
#[test]
fn scram_client_proof_matches_rfc7677() {
    let program = format!(
        "## Main\n\
         Let authMsg be text_bytes(\"{RFC7677_AUTH_MESSAGE}\").\n\
         Let sSalt be pgBase64Decode(\"W22ZaJ0SNY7soEsUEjb6gQ==\").\n\
         Show pgBase64(scramClientProof(text_bytes(\"pencil\"), sSalt, 4096, authMsg)).\n"
    );
    assert_all_tiers(&program, "dHzbZapWIk4jUhN+Ute9ytag9zjfMHgsqmmiz7AndVQ=");
}

/// ServerSignature = HMAC(ServerKey, AuthMessage) with ServerKey =
/// HMAC(SaltedPassword, "Server Key") — the RFC 7677 v= value.
#[test]
fn scram_server_signature_matches_rfc7677() {
    let program = format!(
        "## Main\n\
         Let authMsg be text_bytes(\"{RFC7677_AUTH_MESSAGE}\").\n\
         Let sSalt be pgBase64Decode(\"W22ZaJ0SNY7soEsUEjb6gQ==\").\n\
         Show pgBase64(scramServerSignature(text_bytes(\"pencil\"), sSalt, 4096, authMsg)).\n"
    );
    assert_all_tiers(&program, "6rriTRBi23WpRR/wtup+mMhUZUn/dB5nLTJRsjl95G4=");
}

/// The SCRAM client nonce: 18 random bytes base64-encoded — exactly 24
/// printable characters, never a comma (SCRAM's attribute separator), never a
/// NUL. The VALUE is random; these properties are law on BOTH tiers, which
/// also locks the randomInt native seam the VM shares with the tree-walker.
#[test]
fn pg_scram_client_nonce_shape_holds_on_all_tiers() {
    assert_all_tiers(
        "## Main\n\
         Let nonce be pgScramClientNonce().\n\
         Let nb be text_bytes(nonce).\n\
         Show length of nb.\n\
         Let mutable bad be 0.\n\
         Repeat for b in nb:\n\
         \x20   If b is equal to 44:\n\
         \x20       Set bad to bad + 1.\n\
         \x20   If b is equal to 0:\n\
         \x20       Set bad to bad + 1.\n\
         \x20   If b is less than 33:\n\
         \x20       Set bad to bad + 1.\n\
         \x20   If b is greater than 126:\n\
         \x20       Set bad to bad + 1.\n\
         Show bad.\n",
        "24\n0",
    );
}

// ---------------------------------------------------------------------------
// CommandComplete tag → rows affected (PQcmdTuples semantics)
// ---------------------------------------------------------------------------

/// The rows-affected count is the LAST space-separated token of the tag when
/// numeric ("INSERT 0 3" → 3, "UPDATE 7" → 7, "SELECT 5" → 5); tags without a
/// trailing count ("CREATE TABLE") report 0.
#[test]
fn pg_rows_affected_parses_command_tags() {
    assert_all_tiers(
        "## Main\n\
         Show pgRowsAffected(\"INSERT 0 3\").\n\
         Show pgRowsAffected(\"UPDATE 7\").\n\
         Show pgRowsAffected(\"DELETE 0\").\n\
         Show pgRowsAffected(\"SELECT 5\").\n\
         Show pgRowsAffected(\"CREATE TABLE\").\n\
         Show pgRowsAffected(\"BEGIN\").\n\
         Show pgRowsAffected(\"\").\n",
        "3\n7\n0\n5\n0\n0\n0",
    );
}

// ---------------------------------------------------------------------------
// Connection-URL parsing (postgres:// and postgresql://), pure LOGOS
// ---------------------------------------------------------------------------

/// The full URL form: scheme, user, password, host, port, database.
#[test]
fn pg_parse_url_full_form() {
    assert_all_tiers(
        "## Main\n\
         Let r be pgParseUrl(\"postgres://alice:secret@db.example.com:5433/appdb\").\n\
         Inspect r:\n\
         \x20   When Ok (u):\n\
         \x20       Show u's user.\n\
         \x20       Show u's pass.\n\
         \x20       Show u's host.\n\
         \x20       Show u's port.\n\
         \x20       Show u's database.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n",
        "alice\nsecret\ndb.example.com\n5433\nappdb",
    );
}

/// The postgresql:// alias parses identically.
#[test]
fn pg_parse_url_postgresql_alias() {
    assert_all_tiers(
        "## Main\n\
         Let r be pgParseUrl(\"postgresql://bob:pw@localhost:5432/mydb\").\n\
         Inspect r:\n\
         \x20   When Ok (u):\n\
         \x20       Show u's user.\n\
         \x20       Show u's host.\n\
         \x20       Show u's port.\n\
         \x20       Show u's database.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n",
        "bob\nlocalhost\n5432\nmydb",
    );
}

/// Defaults: no port → 5432; no database path → the user name (libpq's
/// dbname-defaults-to-user rule); no password → "".
#[test]
fn pg_parse_url_defaults() {
    assert_all_tiers(
        "## Main\n\
         Let r be pgParseUrl(\"postgres://carol@dbhost\").\n\
         Inspect r:\n\
         \x20   When Ok (u):\n\
         \x20       Show u's user.\n\
         \x20       Show u's pass.\n\
         \x20       Show u's host.\n\
         \x20       Show u's port.\n\
         \x20       Show u's database.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n",
        "carol\n\ndbhost\n5432\ncarol",
    );
}

/// A wrong scheme is a value-level Err, never a trap.
#[test]
fn pg_parse_url_rejects_a_wrong_scheme() {
    assert_all_tiers(
        "## Main\n\
         Let r be pgParseUrl(\"mysql://alice@host/db\").\n\
         Inspect r:\n\
         \x20   When Ok (u):\n\
         \x20       Show \"unexpectedly parsed\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n",
        "pgParseUrl: the url must start with postgres:// or postgresql://",
    );
}

/// A missing user is an Err (our client always authenticates as someone).
#[test]
fn pg_parse_url_rejects_a_missing_user() {
    assert_all_tiers(
        "## Main\n\
         Let r be pgParseUrl(\"postgres://dbhost:5432/db\").\n\
         Inspect r:\n\
         \x20   When Ok (u):\n\
         \x20       Show \"unexpectedly parsed\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n",
        "pgParseUrl: the url must include a user (postgres://user@host)",
    );
}

/// A non-numeric port is an Err, never a trap.
#[test]
fn pg_parse_url_rejects_a_bad_port() {
    assert_all_tiers(
        "## Main\n\
         Let r be pgParseUrl(\"postgres://alice@dbhost:banana/db\").\n\
         Inspect r:\n\
         \x20   When Ok (u):\n\
         \x20       Show \"unexpectedly parsed\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n",
        "pgParseUrl: the port must be a number",
    );
}

// ---------------------------------------------------------------------------
// Rust-side differentials — the postgres-protocol crate encodes the SAME hex
// ---------------------------------------------------------------------------

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/assets/pg")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("missing fixture {}: {e}", path.display()))
        .trim()
        .to_string()
}

/// THREE-WAY AGREEMENT, leg 2: the postgres-protocol crate (the reference Rust
/// implementation of the wire encoding) produces the SAME startup bytes the
/// LOGOS locks above pin — for both the (u, d) doc vector and the (alice,
/// appdb) fixture the scripted-server suite replays.
#[test]
fn postgres_protocol_crate_agrees_on_startup_bytes() {
    use bytes::BytesMut;
    use postgres_protocol::message::frontend;

    let mut buf = BytesMut::new();
    frontend::startup_message([("user", "u"), ("database", "d")], &mut buf)
        .expect("startup encodes");
    assert_eq!(
        hex(&buf),
        "0000001b0003000075736572007500646174616261736500640000",
        "postgres-protocol disagrees with the pinned (u, d) StartupMessage hex"
    );

    let mut buf = BytesMut::new();
    frontend::startup_message([("user", "alice"), ("database", "appdb")], &mut buf)
        .expect("startup encodes");
    assert_eq!(
        hex(&buf),
        fixture("startup_alice_appdb.hex"),
        "postgres-protocol disagrees with the checked-in startup fixture"
    );
}

/// THREE-WAY AGREEMENT, leg 2 for Query('Q'), PasswordMessage('p'), and
/// Terminate('X') — byte-identical to the LOGOS locks and the fixtures.
#[test]
fn postgres_protocol_crate_agrees_on_query_password_terminate_bytes() {
    use bytes::BytesMut;
    use postgres_protocol::message::frontend;

    let mut buf = BytesMut::new();
    frontend::query("SELECT 1", &mut buf).expect("query encodes");
    assert_eq!(hex(&buf), "510000000d53454c454354203100");

    let mut buf = BytesMut::new();
    frontend::query("SELECT id, name FROM t", &mut buf).expect("query encodes");
    assert_eq!(hex(&buf), fixture("query_select_id_name.hex"));

    let mut buf = BytesMut::new();
    frontend::password_message(b"secret", &mut buf).expect("password encodes");
    assert_eq!(hex(&buf), "700000000b73656372657400");
    assert_eq!(hex(&buf), fixture("password_cleartext_secret.hex"));

    let mut buf = BytesMut::new();
    frontend::terminate(&mut buf);
    assert_eq!(hex(&buf), "5800000004");
    assert_eq!(hex(&buf), fixture("terminate.hex"));
}

/// The MD5 fixture the scripted server asserts is the same construction
/// postgres-protocol's authentication helper computes: md5(md5(pw+user)+salt),
/// "md5"-prefixed, framed as a PasswordMessage.
#[test]
fn postgres_protocol_crate_agrees_on_the_md5_password_fixture() {
    use bytes::BytesMut;
    use postgres_protocol::authentication::md5_hash;
    use postgres_protocol::message::frontend;

    let hashed = md5_hash(b"alice", b"secret", [0x71, 0x77, 0x65, 0x72]);
    assert_eq!(hashed, "md521084184b0b97a0f5996f86647cee7aa");

    let mut buf = BytesMut::new();
    frontend::password_message(hashed.as_bytes(), &mut buf).expect("password encodes");
    assert_eq!(hex(&buf), fixture("password_md5_alice_secret_qwer.hex"));
}

/// The SASLInitialResponse frame agrees with postgres-protocol's encoder for
/// the same fixed nonce payload the LOGOS lock pins.
#[test]
fn postgres_protocol_crate_agrees_on_sasl_frames() {
    use bytes::BytesMut;
    use postgres_protocol::message::frontend;

    let mut buf = BytesMut::new();
    frontend::sasl_initial_response("SCRAM-SHA-256", b"n,,n=,r=fixednoncefixedno", &mut buf)
        .expect("sasl initial encodes");
    assert_eq!(
        hex(&buf),
        "700000002f534352414d2d5348412d32353600000000196e2c2c6e3d2c723d66697865646e6f6e636566697865646e6f"
    );

    let mut buf = BytesMut::new();
    frontend::sasl_response(b"c=biws,r=ab", &mut buf).expect("sasl response encodes");
    assert_eq!(hex(&buf), "700000000f633d626977732c723d6162");
}

// ═══════════════════════════════════════════════════════════════════════════
// G3 — EXTENDED QUERY PROTOCOL frontend messages, verified against the
// PostgreSQL source (the source is golden):
//   tags:    src/include/libpq/protocol.h:19-27 (Parse 'P', Bind 'B',
//            Describe 'D', Execute 'E', Sync 'S')
//   Parse:   src/interfaces/libpq/fe-exec.c:1585-1620 (PQsendPrepare:
//            stmtName cstring + query cstring + Int16 0 param types, then Sync)
//   Bind:    src/interfaces/libpq/fe-exec.c:1824-1884 (portal + stmt +
//            Int16 0 format codes [all text] + Int16 nParams + per param
//            Int32 length/-1-for-NULL + bytes + Int16 1 + Int16 0 result fmt)
//   Describe/Execute: src/interfaces/libpq/fe-exec.c:1890-1901 ('S'/'P' +
//            name; portal + Int32 max_rows — 0 means ALL, backend
//            src/backend/tcop/postgres.c:2317-2318 max_rows <= 0 → FETCH_ALL)
//   Cancel:  src/include/libpq/pqcomm.h:89,122,155-170 (PG_PROTOCOL(1234,5678)
//            = 80877102; 4-byte key in protocol 3.0 → 16-byte packet)
// ═══════════════════════════════════════════════════════════════════════════

/// Parse('P'): statement-name cstring, query cstring, Int16 0 declared param
/// types (the server infers them — exactly what libpq sends when paramTypes is
/// NULL, fe-exec.c:1607-1608). Named fixture + the unnamed-statement form.
#[test]
fn pg_parse_message_is_byte_exact() {
    assert_all_tiers(
        "## Main\n\
         Show bytesToHex(pgParseMessage(\"s1\", \"SELECT $1::int4, $2::text\")).\n\
         Show bytesToHex(pgParseMessage(\"\", \"SELECT 1\")).\n",
        "500000002373310053454c4543542024313a3a696e74342c2024323a3a74657874000000\n\
         50000000100053454c4543542031000000",
    );
}

/// Describe('D'): the variant byte 'S' (statement) or 'P' (portal) then the
/// name cstring — fe-exec.c:1890-1894 and PQsendTypedCommand (fe-exec.c:2476).
#[test]
fn pg_describe_messages_are_byte_exact() {
    assert_all_tiers(
        "## Main\n\
         Show bytesToHex(pgDescribeStatementMessage(\"s1\")).\n\
         Show bytesToHex(pgDescribePortalMessage(\"\")).\n",
        "440000000853733100\n\
         44000000065000",
    );
}

/// Bind('B') in text format, byte-for-byte: portal + statement + Int16 0
/// format codes (all-text default, fe-exec.c:1838-1843) + params (NULL is
/// Int32 -1, fe-exec.c:1876-1880) + Int16 1 + Int16 0 (one text result
/// format). Two text params, a NULL via the nulls mask, zero params, and a
/// multi-byte UTF-8 param whose length must count BYTES, not characters.
#[test]
fn pg_bind_message_is_byte_exact() {
    assert_all_tiers(
        "## Main\n\
         Let ps1: Seq of Text be [\"42\", \"hello\"].\n\
         Let ns1: Seq of Bool be [false, false].\n\
         Show bytesToHex(pgBindMessage(\"\", \"s1\", ps1, ns1)).\n\
         Let ps2: Seq of Text be [\"42\", \"\"].\n\
         Let ns2: Seq of Bool be [false, true].\n\
         Show bytesToHex(pgBindMessage(\"\", \"s1\", ps2, ns2)).\n\
         Let ps3 be a new Seq of Text.\n\
         Let ns3 be a new Seq of Bool.\n\
         Show bytesToHex(pgBindMessage(\"\", \"s1\", ps3, ns3)).\n\
         Let ps4: Seq of Text be [\"héllo\"].\n\
         Let ns4: Seq of Bool be [false].\n\
         Show bytesToHex(pgBindMessage(\"\", \"s1\", ps4, ns4)).\n",
        "420000001f00733100000000020000000234320000000568656c6c6f00010000\n\
         420000001a0073310000000002000000023432ffffffff00010000\n\
         4200000010007331000000000000010000\n\
         420000001a00733100000000010000000668c3a96c6c6f00010000",
    );
}

/// A nulls mask SHORTER than the params treats the missing entries as
/// not-NULL — the builder is total (the Result-typed callers validate
/// lengths before building).
#[test]
fn pg_bind_message_short_mask_means_not_null() {
    assert_all_tiers(
        "## Main\n\
         Let ps: Seq of Text be [\"42\", \"hello\"].\n\
         Let ns be a new Seq of Bool.\n\
         Show bytesToHex(pgBindMessage(\"\", \"s1\", ps, ns)).\n",
        "420000001f00733100000000020000000234320000000568656c6c6f00010000",
    );
}

/// Execute('E'): portal cstring + Int32 max_rows. We always send 0 = fetch
/// ALL rows (postgres.c:2317-2318), so PortalSuspended ('s') never arrives;
/// the named-portal + row-limit form is locked for encoder generality.
#[test]
fn pg_execute_message_is_byte_exact() {
    assert_all_tiers(
        "## Main\n\
         Show bytesToHex(pgExecuteMessage(\"\", 0)).\n\
         Show bytesToHex(pgExecuteMessage(\"p1\", 7)).\n",
        "45000000090000000000\n\
         450000000b70310000000007",
    );
}

/// Sync('S') is an empty-body tagged message: exactly 53 00 00 00 04
/// (fe-exec.c:1906-1908).
#[test]
fn pg_sync_message_is_byte_exact() {
    assert_all_tiers(
        "## Main\n\
         Show bytesToHex(pgSyncMessage()).\n",
        "5300000004",
    );
}

/// The CancelRequest packet: Int32 16 (total length), Int32 80877102
/// (PG_PROTOCOL(1234,5678) = 1234<<16 | 5678, pqcomm.h:89,122), Int32
/// backend PID, Int32 secret key — 16 bytes, UNTAGGED, on a FRESH
/// connection. The pid/secret pair matches the scripted BackendKeyData.
#[test]
fn pg_cancel_request_message_is_byte_exact() {
    assert_all_tiers(
        "## Main\n\
         Show bytesToHex(pgCancelRequestMessage(18, 52)).\n\
         Show bytesToHex(pgCancelRequestMessage(2147483647, 0)).\n",
        "0000001004d2162e0000001200000034\n\
         0000001004d2162e7fffffff00000000",
    );
}

// ---------------------------------------------------------------------------
// G3 — type OIDs, verified against src/include/catalog/pg_type.dat:
// bool=16 (line 34), int8=20 (55), int2=21 (60), int4=23 (72), text=25 (81),
// float8=701 (223), varchar=1043 (280), numeric=1700 (347), uuid=2950 (408)
// ---------------------------------------------------------------------------

#[test]
fn pg_oid_constants_match_pg_type_dat() {
    assert_all_tiers(
        "## Main\n\
         Show pgOidBool().\n\
         Show pgOidInt8().\n\
         Show pgOidInt2().\n\
         Show pgOidInt4().\n\
         Show pgOidText().\n\
         Show pgOidFloat8().\n\
         Show pgOidVarchar().\n\
         Show pgOidNumeric().\n\
         Show pgOidUuid().\n",
        "16\n20\n21\n23\n25\n701\n1043\n1700\n2950",
    );
}

// ---------------------------------------------------------------------------
// G3 — pure backend-body parsers (RowDescription 'T', DataRow 'D',
// ParameterDescription 't'), lifted out of the receive loops so they are
// lockable and fuzzable WITHOUT a socket
// ---------------------------------------------------------------------------

/// Renders a backend-message BODY (no tag, no length) as a LOGOS byte-list
/// literal, so the pure parsers can be driven from canned frames.
fn logos_bytes(body: &[u8]) -> String {
    let nums: Vec<String> = body.iter().map(|b| b.to_string()).collect();
    format!("[{}]", nums.join(", "))
}

/// RowDescription BODY for (name, oid) pairs — the same layout the scripted
/// servers send: Int16 count, then per field name cstring + table oid Int32 +
/// attnum Int16 + type oid Int32 + typlen Int16 + atttypmod Int32 + format
/// Int16 (docs "Message Formats", RowDescription).
fn row_description_body(cols: &[(&str, i32)]) -> Vec<u8> {
    let mut body = (cols.len() as i16).to_be_bytes().to_vec();
    for (name, oid) in cols {
        body.extend_from_slice(name.as_bytes());
        body.push(0);
        body.extend_from_slice(&0i32.to_be_bytes());
        body.extend_from_slice(&0i16.to_be_bytes());
        body.extend_from_slice(&oid.to_be_bytes());
        body.extend_from_slice(&(-1i16).to_be_bytes());
        body.extend_from_slice(&(-1i32).to_be_bytes());
        body.extend_from_slice(&0i16.to_be_bytes());
    }
    body
}

/// DataRow BODY: Int16 count, then per cell Int32 length + bytes, or
/// Int32 -1 for NULL (docs "Message Formats", DataRow).
fn data_row_body(cells: &[Option<&str>]) -> Vec<u8> {
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
    body
}

/// ParameterDescription BODY: Int16 count + Int32 oids (docs "Message
/// Formats", ParameterDescription; sent by exec_describe_statement_message,
/// src/backend/tcop/postgres.c:2726).
fn param_description_body(oids: &[i32]) -> Vec<u8> {
    let mut body = (oids.len() as i16).to_be_bytes().to_vec();
    for oid in oids {
        body.extend_from_slice(&oid.to_be_bytes());
    }
    body
}

/// pgParseRowDescription reads names + type OIDs from a RowDescription body.
#[test]
fn pg_parse_row_description_reads_names_and_oids() {
    let body = row_description_body(&[("id", 23), ("name", 25), ("née", 1700)]);
    let program = format!(
        "## Main\n\
         Let body be {}.\n\
         Let (names, oids) be pgParseRowDescription(body).\n\
         Repeat for nm in names:\n\
         \x20   Show \"<{{nm}}>\".\n\
         Show oids.\n",
        logos_bytes(&body)
    );
    assert_all_tiers(&program, "<id>\n<name>\n<née>\n[23, 25, 1700]");
}

/// pgParseDataRow distinguishes NULL (length -1 → mask true) from the EMPTY
/// STRING (length 0 → mask false) — the heart of the honest-NULL upgrade.
#[test]
fn pg_parse_data_row_distinguishes_null_from_empty() {
    let body = data_row_body(&[Some("a"), None, Some(""), Some("héllo")]);
    let program = format!(
        "## Main\n\
         Let body be {}.\n\
         Let (cells, mask) be pgParseDataRow(body).\n\
         Repeat for cell in cells:\n\
         \x20   Show \"<{{cell}}>\".\n\
         Repeat for b in mask:\n\
         \x20   Show b.\n",
        logos_bytes(&body)
    );
    assert_all_tiers(&program, "<a>\n<>\n<>\n<héllo>\nfalse\ntrue\nfalse\nfalse");
}

/// pgParseParamDescription reads the declared parameter OIDs.
#[test]
fn pg_parse_param_description_reads_oids() {
    let body = param_description_body(&[23, 25, 1700]);
    let program = format!(
        "## Main\n\
         Let body be {}.\n\
         Show pgParseParamDescription(body).\n",
        logos_bytes(&body)
    );
    assert_all_tiers(&program, "[23, 25, 1700]");
}

// ---------------------------------------------------------------------------
// G3 — typed values over an honest NULL mask. QueryResult is constructed
// LITERALLY here (no socket): the getters are pure functions of the struct.
// ---------------------------------------------------------------------------

/// Builds a LOGOS preamble constructing a QueryResult with the given columns,
/// rows (None = NULL), and command tag. Mirrors what the receive loops build.
fn qr_preamble(cols: &[(&str, i32)], rows: &[&[Option<&str>]], cmd_tag: &str) -> String {
    let mut p = String::from("Let mutable cols be a new Seq of Text.\n");
    for (name, _) in cols {
        p.push_str(&format!("Push \"{name}\" to cols.\n"));
    }
    p.push_str("Let mutable oids be a new Seq of Int.\n");
    for (_, oid) in cols {
        p.push_str(&format!("Push {oid} to oids.\n"));
    }
    p.push_str("Let mutable rws be a new Seq of Seq of Text.\n");
    p.push_str("Let mutable masks be a new Seq of Seq of Bool.\n");
    for (i, row) in rows.iter().enumerate() {
        p.push_str(&format!("Let mutable r{i} be a new Seq of Text.\n"));
        p.push_str(&format!("Let mutable m{i} be a new Seq of Bool.\n"));
        for cell in row.iter() {
            match cell {
                None => {
                    p.push_str(&format!("Push \"\" to r{i}.\n"));
                    p.push_str(&format!("Push true to m{i}.\n"));
                }
                Some(v) => {
                    p.push_str(&format!("Push \"{v}\" to r{i}.\n"));
                    p.push_str(&format!("Push false to m{i}.\n"));
                }
            }
        }
        p.push_str(&format!("Push r{i} to rws.\n"));
        p.push_str(&format!("Push m{i} to masks.\n"));
    }
    p.push_str(&format!(
        "Let qr be a new QueryResult with columns cols and typeOids oids and rows rws and nullMask masks and cmdTag \"{cmd_tag}\".\n"
    ));
    p
}

/// A canned two-column, two-row result: (id int4, name text) with row 2's
/// name NULL — used by the mask/Opt-getter locks below.
fn qr_two_rows() -> String {
    qr_preamble(
        &[("id", 23), ("name", 25)],
        &[&[Some("1"), Some("alice")], &[Some("2"), None]],
        "SELECT 2",
    )
}

/// pgIsNullAt reads the mask, bounds-checked with the SAME range errors as
/// the cell getters.
#[test]
fn pg_is_null_at_reads_the_mask() {
    let program = format!(
        "## Main\n{}\
         Let a be pgIsNullAt(qr, 1, 2).\n\
         Inspect a:\n\
         \x20   When Ok (v):\n\
         \x20       Show v.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Let b be pgIsNullAt(qr, 2, 2).\n\
         Inspect b:\n\
         \x20   When Ok (v):\n\
         \x20       Show v.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Let c be pgIsNullAt(qr, 3, 1).\n\
         Inspect c:\n\
         \x20   When Ok (v):\n\
         \x20       Show v.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Let d be pgIsNullAt(qr, 1, 3).\n\
         Inspect d:\n\
         \x20   When Ok (v):\n\
         \x20       Show v.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n",
        qr_two_rows()
    );
    assert_all_tiers(
        &program,
        "false\ntrue\npg: row 3 is out of range (1..2)\npg: column 3 is out of range (1..2)",
    );
}

/// The Opt getters return Ok(some v) for a value, Ok(none) for NULL, and the
/// range Err for out-of-bounds — NULL is now expressible, not spoofed as "".
#[test]
fn pg_text_at_opt_some_none_and_range() {
    let program = format!(
        "## Main\n{}\
         Let a be pgTextAtOpt(qr, 1, 2).\n\
         Inspect a:\n\
         \x20   When Ok (ov):\n\
         \x20       Inspect ov:\n\
         \x20           When Some (v):\n\
         \x20               Show \"some <{{v}}>\".\n\
         \x20           When None:\n\
         \x20               Show \"none\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Let b be pgTextAtOpt(qr, 2, 2).\n\
         Inspect b:\n\
         \x20   When Ok (ov):\n\
         \x20       Inspect ov:\n\
         \x20           When Some (v):\n\
         \x20               Show \"some <{{v}}>\".\n\
         \x20           When None:\n\
         \x20               Show \"none\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Let c be pgTextAtOpt(qr, 3, 2).\n\
         Inspect c:\n\
         \x20   When Ok (ov):\n\
         \x20       Inspect ov:\n\
         \x20           When Some (v):\n\
         \x20               Show \"some <{{v}}>\".\n\
         \x20           When None:\n\
         \x20               Show \"none\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n",
        qr_two_rows()
    );
    assert_all_tiers(
        &program,
        "some <alice>\nnone\npg: row 3 is out of range (1..2)",
    );
}

/// pgIntAtOpt: a NULL int4 is Ok(none); a present one is Ok(some n); a
/// non-integer cell is still the parse Err.
#[test]
fn pg_int_at_opt_some_none_and_parse_err() {
    let program = format!(
        "## Main\n{}\
         Let a be pgIntAtOpt(qr, 1, 1).\n\
         Inspect a:\n\
         \x20   When Ok (ov):\n\
         \x20       Inspect ov:\n\
         \x20           When Some (v):\n\
         \x20               Show v.\n\
         \x20           When None:\n\
         \x20               Show \"none\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Let b be pgIntAtOpt(qr, 2, 2).\n\
         Inspect b:\n\
         \x20   When Ok (ov):\n\
         \x20       Inspect ov:\n\
         \x20           When Some (v):\n\
         \x20               Show v.\n\
         \x20           When None:\n\
         \x20               Show \"none\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Let c be pgIntAtOpt(qr, 1, 2).\n\
         Inspect c:\n\
         \x20   When Ok (ov):\n\
         \x20       Inspect ov:\n\
         \x20           When Some (v):\n\
         \x20               Show v.\n\
         \x20           When None:\n\
         \x20               Show \"none\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n",
        qr_two_rows()
    );
    assert_all_tiers(&program, "1\nnone\npg: the value 'alice' is not an Int");
}

/// pgBoolAtOpt and pgRealAtOpt: some / none / parse-Err, same discipline.
#[test]
fn pg_bool_and_real_at_opt() {
    let preamble = qr_preamble(
        &[("ok", 16), ("score", 701)],
        &[&[Some("t"), Some("-0.25")], &[None, None]],
        "SELECT 2",
    );
    let program = format!(
        "## Main\n{preamble}\
         Let a be pgBoolAtOpt(qr, 1, 1).\n\
         Inspect a:\n\
         \x20   When Ok (ov):\n\
         \x20       Inspect ov:\n\
         \x20           When Some (v):\n\
         \x20               Show v.\n\
         \x20           When None:\n\
         \x20               Show \"none\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Let b be pgBoolAtOpt(qr, 2, 1).\n\
         Inspect b:\n\
         \x20   When Ok (ov):\n\
         \x20       Inspect ov:\n\
         \x20           When Some (v):\n\
         \x20               Show v.\n\
         \x20           When None:\n\
         \x20               Show \"none\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Let c be pgRealAtOpt(qr, 1, 2).\n\
         Inspect c:\n\
         \x20   When Ok (ov):\n\
         \x20       Inspect ov:\n\
         \x20           When Some (v):\n\
         \x20               Show v.\n\
         \x20           When None:\n\
         \x20               Show \"none\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Let d be pgRealAtOpt(qr, 2, 2).\n\
         Inspect d:\n\
         \x20   When Ok (ov):\n\
         \x20       Inspect ov:\n\
         \x20           When Some (v):\n\
         \x20               Show v.\n\
         \x20           When None:\n\
         \x20               Show \"none\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Let f be pgBoolAtOpt(qr, 1, 2).\n\
         Inspect f:\n\
         \x20   When Ok (ov):\n\
         \x20       Inspect ov:\n\
         \x20           When Some (v):\n\
         \x20               Show v.\n\
         \x20           When None:\n\
         \x20               Show \"none\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n",
        preamble = preamble
    );
    assert_all_tiers(
        &program,
        "true\nnone\n-0.25\nnone\npg: the value '-0.25' is not a Bool",
    );
}

/// The LEGACY getters keep their A3 contract, now re-locked EXPLICITLY as the
/// documented projection: pgTextAt spoofs NULL as "" (use the Opt getters for
/// honesty), and the typed parsers reject the "" a NULL projects to.
#[test]
fn legacy_getters_keep_the_a3_projection() {
    let program = format!(
        "## Main\n{}\
         Let a be pgTextAt(qr, 2, 2).\n\
         Inspect a:\n\
         \x20   When Ok (v):\n\
         \x20       Show \"<{{v}}>\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Let b be pgIntAt(qr, 2, 2).\n\
         Inspect b:\n\
         \x20   When Ok (v):\n\
         \x20       Show v.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Let c be pgBoolAt(qr, 2, 2).\n\
         Inspect c:\n\
         \x20   When Ok (v):\n\
         \x20       Show v.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n",
        qr_two_rows()
    );
    assert_all_tiers(
        &program,
        "<>\npg: the value '' is not an Int\npg: the value '' is not a Bool",
    );
}

/// pgDecimalAt parses numeric text into the EXACT decimal value — 30+ digit
/// values round-trip digit-for-digit (numeric_out emits plain decimal
/// notation, src/backend/utils/adt/numeric.c get_str_from_var). NaN and a
/// NULL cell are value-level Errs, never traps.
#[test]
fn pg_decimal_at_is_exact_for_30_plus_digits() {
    let big = "123456789012345678901234567890.123456789012345678901234567890";
    let preamble = qr_preamble(
        &[("x", 1700)],
        &[
            &[Some(big)],
            &[Some("-0.00000000000000000000000000001")],
            &[Some("NaN")],
            &[None],
        ],
        "SELECT 4",
    );
    let program = format!(
        "## Main\n{preamble}\
         Let a be pgDecimalAt(qr, 1, 1).\n\
         Inspect a:\n\
         \x20   When Ok (v):\n\
         \x20       Show v.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Let b be pgDecimalAt(qr, 2, 1).\n\
         Inspect b:\n\
         \x20   When Ok (v):\n\
         \x20       Show v.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Let c be pgDecimalAt(qr, 3, 1).\n\
         Inspect c:\n\
         \x20   When Ok (v):\n\
         \x20       Show v.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Let d be pgDecimalAt(qr, 4, 1).\n\
         Inspect d:\n\
         \x20   When Ok (v):\n\
         \x20       Show v.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n",
        preamble = preamble
    );
    assert_all_tiers(
        &program,
        &format!(
            "{big}\n\
             -0.00000000000000000000000000001\n\
             pg: the value 'NaN' is not a numeric\n\
             pg: the cell at row 4 column 1 is NULL (use pgDecimalAtOpt)"
        ),
    );
}

/// pgDecimalAtOpt: NULL is Ok(none), a value is Ok(some d).
#[test]
fn pg_decimal_at_opt_some_none() {
    let preamble = qr_preamble(&[("x", 1700)], &[&[Some("19.99")], &[None]], "SELECT 2");
    let program = format!(
        "## Main\n{preamble}\
         Let a be pgDecimalAtOpt(qr, 1, 1).\n\
         Inspect a:\n\
         \x20   When Ok (ov):\n\
         \x20       Inspect ov:\n\
         \x20           When Some (v):\n\
         \x20               Show v.\n\
         \x20           When None:\n\
         \x20               Show \"none\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Let b be pgDecimalAtOpt(qr, 2, 1).\n\
         Inspect b:\n\
         \x20   When Ok (ov):\n\
         \x20       Inspect ov:\n\
         \x20           When Some (v):\n\
         \x20               Show v.\n\
         \x20           When None:\n\
         \x20               Show \"none\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n",
        preamble = preamble
    );
    assert_all_tiers(&program, "19.99\nnone");
}

/// pgUuidAt parses the canonical 8-4-4-4-12 text (uuid_out,
/// src/backend/utils/adt/uuid.c) into a Uuid, normalizing case; a malformed
/// cell and a NULL cell are Errs.
#[test]
fn pg_uuid_at_parses_and_normalizes() {
    let preamble = qr_preamble(
        &[("u", 2950)],
        &[
            &[Some("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11")],
            &[Some("A0EEBC99-9C0B-4EF8-BB6D-6BB9BD380A11")],
            &[Some("not-a-uuid")],
            &[None],
        ],
        "SELECT 4",
    );
    let program = format!(
        "## Main\n{preamble}\
         Let a be pgUuidAt(qr, 1, 1).\n\
         Inspect a:\n\
         \x20   When Ok (v):\n\
         \x20       Show uuidFormat(v).\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Let b be pgUuidAt(qr, 2, 1).\n\
         Inspect b:\n\
         \x20   When Ok (v):\n\
         \x20       Show uuidFormat(v).\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Let c be pgUuidAt(qr, 3, 1).\n\
         Inspect c:\n\
         \x20   When Ok (v):\n\
         \x20       Show uuidFormat(v).\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Let d be pgUuidAt(qr, 4, 1).\n\
         Inspect d:\n\
         \x20   When Ok (v):\n\
         \x20       Show uuidFormat(v).\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n",
        preamble = preamble
    );
    assert_all_tiers(
        &program,
        "a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11\n\
         a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11\n\
         pg: the value 'not-a-uuid' is not a uuid\n\
         pg: the cell at row 4 column 1 is NULL (use pgUuidAtOpt)",
    );
}

/// pgEnumAt asserts membership in the expected label set (PG enum values
/// arrive as text); a label outside the set and a NULL are Errs, and
/// pgEnumAtOpt makes NULL expressible.
#[test]
fn pg_enum_at_checks_membership() {
    let preamble = qr_preamble(
        &[("mood", 99999)],
        &[&[Some("happy")], &[Some("grumpy")], &[None]],
        "SELECT 3",
    );
    let program = format!(
        "## Main\n{preamble}\
         Let allowed: Seq of Text be [\"sad\", \"ok\", \"happy\"].\n\
         Let a be pgEnumAt(qr, 1, 1, allowed).\n\
         Inspect a:\n\
         \x20   When Ok (v):\n\
         \x20       Show v.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Let b be pgEnumAt(qr, 2, 1, allowed).\n\
         Inspect b:\n\
         \x20   When Ok (v):\n\
         \x20       Show v.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Let c be pgEnumAt(qr, 3, 1, allowed).\n\
         Inspect c:\n\
         \x20   When Ok (v):\n\
         \x20       Show v.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Let d be pgEnumAtOpt(qr, 3, 1, allowed).\n\
         Inspect d:\n\
         \x20   When Ok (ov):\n\
         \x20       Inspect ov:\n\
         \x20           When Some (v):\n\
         \x20               Show v.\n\
         \x20           When None:\n\
         \x20               Show \"none\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n",
        preamble = preamble
    );
    assert_all_tiers(
        &program,
        "happy\n\
         pg: the value 'grumpy' is not one of the expected enum values\n\
         pg: the cell at row 3 column 1 is NULL (use pgEnumAtOpt)\n\
         none",
    );
}

/// pgTypedAt dispatches on the column's declared OID and VALIDATES the cell
/// against it: ints must be integers, bools normalize to true/false, float8
/// must be a real, numeric must be plain decimal, uuid normalizes to
/// lowercase; text/varchar and unknown OIDs pass through opaque; NULL is an
/// Err pointing at pgIsNullAt.
#[test]
fn pg_typed_at_dispatches_on_oid() {
    let preamble = qr_preamble(
        &[
            ("a", 23),
            ("b", 16),
            ("c", 701),
            ("d", 1700),
            ("e", 2950),
            ("f", 25),
            ("g", 424242),
        ],
        &[&[
            Some("42"),
            Some("t"),
            Some("-0.25"),
            Some("19.99"),
            Some("A0EEBC99-9C0B-4EF8-BB6D-6BB9BD380A11"),
            Some("anything at all"),
            Some("opaque"),
        ]],
        "SELECT 1",
    );
    let program = format!(
        "## Main\n{preamble}\
         Repeat for col from 1 to 7:\n\
         \x20   Let t be pgTypedAt(qr, 1, col).\n\
         \x20   Inspect t:\n\
         \x20       When Ok (v):\n\
         \x20           Show \"<{{v}}>\".\n\
         \x20       When Err (e):\n\
         \x20           Show e.\n",
        preamble = preamble
    );
    assert_all_tiers(
        &program,
        "<42>\n<true>\n<-0.25>\n<19.99>\n<a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11>\n\
         <anything at all>\n<opaque>",
    );
}

/// pgTypedAt REJECTS a cell that does not parse as its declared type, and a
/// NULL cell.
#[test]
fn pg_typed_at_rejects_type_lies_and_null() {
    let preamble = qr_preamble(
        &[("a", 23), ("b", 16)],
        &[&[Some("forty-two"), None]],
        "SELECT 1",
    );
    let program = format!(
        "## Main\n{preamble}\
         Let a be pgTypedAt(qr, 1, 1).\n\
         Inspect a:\n\
         \x20   When Ok (v):\n\
         \x20       Show v.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Let b be pgTypedAt(qr, 1, 2).\n\
         Inspect b:\n\
         \x20   When Ok (v):\n\
         \x20       Show v.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n",
        preamble = preamble
    );
    assert_all_tiers(
        &program,
        "pg: the value 'forty-two' is not an Int\n\
         pg: the cell at row 1 column 2 is NULL (use pgIsNullAt)",
    );
}

/// Rows affected for the PARAMETERIZED path: the CommandComplete tag now
/// rides on QueryResult as cmdTag, so prepared DML reports its count through
/// the same PQcmdTuples rule the simple path locks.
#[test]
fn pg_cmd_tag_reports_rows_affected() {
    let preamble = qr_preamble(&[], &[], "INSERT 0 3");
    let program = format!(
        "## Main\n{preamble}\
         Let tagT be qr's cmdTag.\n\
         Show tagT.\n\
         Show pgRowsAffected(tagT).\n",
        preamble = preamble
    );
    assert_all_tiers(&program, "INSERT 0 3\n3");
}

// ---------------------------------------------------------------------------
// G3 — Rust-side differentials for the NEW messages: the postgres-protocol
// crate encodes the SAME bytes as the LOGOS locks and the fixtures
// ---------------------------------------------------------------------------

/// THREE-WAY AGREEMENT for the extended protocol: Parse, Describe(S/P),
/// Bind (text params + NULL), Execute, and Sync — the reference encoder
/// reproduces every fixture the LOGOS locks pin.
#[test]
fn postgres_protocol_crate_agrees_on_extended_protocol_bytes() {
    use bytes::BytesMut;
    use postgres_protocol::message::frontend;
    use postgres_protocol::IsNull;

    let mut buf = BytesMut::new();
    frontend::parse("s1", "SELECT $1::int4, $2::text", [], &mut buf).expect("parse encodes");
    assert_eq!(hex(&buf), fixture("parse_named_s1.hex"));

    let mut buf = BytesMut::new();
    frontend::describe(b'S', "s1", &mut buf).expect("describe S encodes");
    assert_eq!(hex(&buf), fixture("describe_statement_s1.hex"));

    let mut buf = BytesMut::new();
    frontend::describe(b'P', "", &mut buf).expect("describe P encodes");
    assert_eq!(hex(&buf), fixture("describe_portal_unnamed.hex"));

    let serialize = |v: &Option<&str>, buf: &mut BytesMut| match v {
        None => Ok(IsNull::Yes),
        Some(s) => {
            buf.extend_from_slice(s.as_bytes());
            Ok(IsNull::No)
        }
    };

    let mut buf = BytesMut::new();
    frontend::bind(
        "",
        "s1",
        [],
        [Some("42"), Some("hello")].iter(),
        |v, buf| serialize(v, buf),
        [0i16],
        &mut buf,
    )
    .unwrap_or_else(|_| panic!("the reference bind encoder failed"));
    assert_eq!(hex(&buf), fixture("bind_s1_params_42_hello.hex"));

    let mut buf = BytesMut::new();
    frontend::bind(
        "",
        "s1",
        [],
        [Some("42"), None].iter(),
        |v, buf| serialize(v, buf),
        [0i16],
        &mut buf,
    )
    .unwrap_or_else(|_| panic!("the reference bind encoder failed on NULL"));
    assert_eq!(hex(&buf), fixture("bind_s1_null_second.hex"));

    let mut buf = BytesMut::new();
    frontend::execute("", 0, &mut buf).expect("execute encodes");
    assert_eq!(hex(&buf), fixture("execute_unnamed_all.hex"));

    let mut buf = BytesMut::new();
    frontend::sync(&mut buf);
    assert_eq!(hex(&buf), fixture("sync.hex"));
}

/// The 16-byte CancelRequest packet agrees with the reference encoder for
/// the scripted pid/secret pair AND for the pinned extremes.
#[test]
fn postgres_protocol_crate_agrees_on_the_cancel_request_packet() {
    use bytes::BytesMut;
    use postgres_protocol::message::frontend;

    let mut buf = BytesMut::new();
    frontend::cancel_request(18, 52, &mut buf);
    assert_eq!(hex(&buf), fixture("cancel_request_pid18_secret52.hex"));

    let mut buf = BytesMut::new();
    frontend::cancel_request(i32::MAX, 0, &mut buf);
    assert_eq!(hex(&buf), "0000001004d2162e7fffffff00000000");
}

// ---------------------------------------------------------------------------
// G3 — deterministic FUZZ differentials. A fixed-seed generator drives random
// shapes through the LOGOS encoders/parsers on BOTH tiers and through the
// postgres-protocol reference implementation; all legs must agree exactly.
// Fixed seeds = reproducible forever; a failure prints the full program.
// ---------------------------------------------------------------------------

/// Deterministic 64-bit LCG (Knuth MMIX constants) — no external RNG, the
/// stream is pinned by the seed and will never drift under dependency bumps.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Characters safe to embed in a LOGOS string literal (no `"`, `\`, `{`, `}`,
/// no control bytes) — including MULTI-BYTE UTF-8 so byte-length vs
/// char-length bugs cannot hide.
const FUZZ_CHARS: &[&str] = &[
    "a", "b", "c", "x", "y", "z", "A", "Z", "0", "1", "9", "_", "-", ".", ",", ":", ";", " ", "!",
    "?", "$", "(", ")", "*", "+", "/", "<", ">", "=", "@", "[", "]", "^", "|", "~", "'", "é", "ß",
    "λ", "Ж", "漢", "字", "🙂",
];

fn fuzz_text(rng: &mut Lcg, max_len: usize) -> String {
    let len = rng.below(max_len + 1);
    (0..len)
        .map(|_| FUZZ_CHARS[rng.below(FUZZ_CHARS.len())])
        .collect()
}

fn fuzz_name(rng: &mut Lcg) -> String {
    const NAME: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789_";
    let len = rng.below(13);
    (0..len)
        .map(|_| NAME[rng.below(NAME.len())] as char)
        .collect()
}

/// FUZZ: Bind frames over random portal/statement names and random param
/// vectors (NULLs, empty strings, multi-byte text) — LOGOS ≡ postgres-protocol
/// on both tiers, 48 shapes.
#[test]
fn fuzz_bind_message_agrees_with_the_reference_encoder() {
    use bytes::BytesMut;
    use postgres_protocol::message::frontend;
    use postgres_protocol::IsNull;

    let mut rng = Lcg(0x09B3_9E37_79B9_7F4A);
    for _chunk in 0..6 {
        let mut program = String::from("## Main\n");
        let mut expected = Vec::new();
        for case in 0..8 {
            let portal = fuzz_name(&mut rng);
            let stmt = fuzz_name(&mut rng);
            let nparams = 1 + rng.below(6);
            let params: Vec<Option<String>> = (0..nparams)
                .map(|_| match rng.below(4) {
                    0 => None,
                    1 => Some(String::new()),
                    _ => Some(fuzz_text(&mut rng, 12)),
                })
                .collect();

            let mut buf = BytesMut::new();
            frontend::bind(
                &portal,
                &stmt,
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
            expected.push(hex(&buf));

            let texts: Vec<String> = params
                .iter()
                .map(|p| format!("\"{}\"", p.as_deref().unwrap_or("")))
                .collect();
            let nulls: Vec<&str> = params
                .iter()
                .map(|p| if p.is_none() { "true" } else { "false" })
                .collect();
            program.push_str(&format!(
                "Let ps{case}: Seq of Text be [{}].\n\
                 Let ns{case}: Seq of Bool be [{}].\n\
                 Show bytesToHex(pgBindMessage(\"{portal}\", \"{stmt}\", ps{case}, ns{case})).\n",
                texts.join(", "),
                nulls.join(", ")
            ));
        }
        assert_all_tiers(&program, &expected.join("\n"));
    }
}

/// FUZZ: Parse and Execute frames over random names, SQL text, and row
/// limits — LOGOS ≡ postgres-protocol, 24 shapes each.
#[test]
fn fuzz_parse_and_execute_messages_agree_with_the_reference_encoder() {
    use bytes::BytesMut;
    use postgres_protocol::message::frontend;

    let mut rng = Lcg(0xCAFE_F00D_D15E_A5E5);
    for _chunk in 0..4 {
        let mut program = String::from("## Main\n");
        let mut expected = Vec::new();
        for _case in 0..6 {
            let name = fuzz_name(&mut rng);
            let sql = fuzz_text(&mut rng, 40);
            let mut buf = BytesMut::new();
            frontend::parse(&name, &sql, [], &mut buf).expect("reference parse encodes");
            expected.push(hex(&buf));
            program.push_str(&format!(
                "Show bytesToHex(pgParseMessage(\"{name}\", \"{sql}\")).\n"
            ));

            let portal = fuzz_name(&mut rng);
            let max_rows = [0i32, 1, 7, 50000, i32::MAX][rng.below(5)];
            let mut buf = BytesMut::new();
            frontend::execute(&portal, max_rows, &mut buf).expect("reference execute encodes");
            expected.push(hex(&buf));
            program.push_str(&format!(
                "Show bytesToHex(pgExecuteMessage(\"{portal}\", {max_rows})).\n"
            ));
        }
        assert_all_tiers(&program, &expected.join("\n"));
    }
}

/// FUZZ: the CancelRequest packet over random pid/secret pairs — LOGOS ≡
/// postgres-protocol, 8 shapes.
#[test]
fn fuzz_cancel_request_agrees_with_the_reference_encoder() {
    use bytes::BytesMut;
    use postgres_protocol::message::frontend;

    let mut rng = Lcg(0x5EED_CA9C_E1BE_EF01);
    let mut program = String::from("## Main\n");
    let mut expected = Vec::new();
    for _case in 0..8 {
        let pid = (rng.next() % (i32::MAX as u64 + 1)) as i32;
        let secret = (rng.next() % (i32::MAX as u64 + 1)) as i32;
        let mut buf = BytesMut::new();
        frontend::cancel_request(pid, secret, &mut buf);
        expected.push(hex(&buf));
        program.push_str(&format!(
            "Show bytesToHex(pgCancelRequestMessage({pid}, {secret})).\n"
        ));
    }
    assert_all_tiers(&program, &expected.join("\n"));
}

/// FUZZ: the DataRow parser — random rows (NULLs, empties, multi-byte text)
/// are built by the generator, parsed by the postgres-protocol BACKEND parser
/// (the reference decoder), and parsed by pgParseDataRow on both tiers; all
/// three agree on cells AND null mask, 24 shapes.
#[test]
fn fuzz_data_row_parser_against_generator_and_reference_backend_parser() {
    use fallible_iterator::FallibleIterator;
    use postgres_protocol::message::backend;

    let mut rng = Lcg(0xDA7A_2013_BEEF_CAFE);
    for _chunk in 0..4 {
        let mut program = String::from("## Main\n");
        let mut expected = Vec::new();
        for case in 0..6 {
            let ncells = 1 + rng.below(6);
            let cells: Vec<Option<String>> = (0..ncells)
                .map(|_| match rng.below(5) {
                    0 => None,
                    1 => Some(String::new()),
                    _ => Some(fuzz_text(&mut rng, 10)),
                })
                .collect();
            let cell_refs: Vec<Option<&str>> = cells.iter().map(|c| c.as_deref()).collect();
            let body = data_row_body(&cell_refs);

            // Reference leg: the postgres-protocol backend parser reads the
            // SAME framed message and must see the same cells and NULLs.
            let mut framed = bytes::BytesMut::new();
            framed.extend_from_slice(b"D");
            framed.extend_from_slice(&((body.len() as i32 + 4).to_be_bytes()));
            framed.extend_from_slice(&body);
            match backend::Message::parse(&mut framed).expect("reference parses the DataRow") {
                Some(backend::Message::DataRow(dr)) => {
                    let ranges: Vec<Option<std::ops::Range<usize>>> =
                        dr.ranges().collect().expect("reference ranges");
                    assert_eq!(ranges.len(), cells.len(), "reference cell count");
                    for (i, r) in ranges.iter().enumerate() {
                        match (r, &cells[i]) {
                            (None, None) => {}
                            (Some(rr), Some(want)) => {
                                assert_eq!(
                                    &dr.buffer()[rr.clone()],
                                    want.as_bytes(),
                                    "reference cell {i} bytes"
                                );
                            }
                            (a, b) => panic!(
                                "reference NULL disagreement at cell {i}: ref-null={} gen-null={}",
                                a.is_none(),
                                b.is_none()
                            ),
                        }
                    }
                }
                _ => panic!("the reference parser did not see a DataRow"),
            }

            program.push_str(&format!(
                "Let body{case} be {}.\n\
                 Let (cells{case}, mask{case}) be pgParseDataRow(body{case}).\n\
                 Repeat for cell in cells{case}:\n\
                 \x20   Show \"<{{cell}}>\".\n\
                 Repeat for mb in mask{case}:\n\
                 \x20   Show mb.\n",
                logos_bytes(&body)
            ));
            for cell in &cells {
                expected.push(format!("<{}>", cell.as_deref().unwrap_or("")));
            }
            for cell in &cells {
                expected.push(if cell.is_none() { "true" } else { "false" }.to_string());
            }
        }
        assert_all_tiers(&program, &expected.join("\n"));
    }
}

/// FUZZ: the RowDescription parser — random column names (multi-byte too) and
/// OIDs through the reference backend parser and pgParseRowDescription, 15
/// shapes.
#[test]
fn fuzz_row_description_parser_against_generator_and_reference_backend_parser() {
    use fallible_iterator::FallibleIterator;
    use postgres_protocol::message::backend;

    let mut rng = Lcg(0x20DE_5C21_9710_2026);
    for _chunk in 0..3 {
        let mut program = String::from("## Main\n");
        let mut expected = Vec::new();
        for case in 0..5 {
            let ncols = 1 + rng.below(5);
            let cols: Vec<(String, i32)> = (0..ncols)
                .map(|_| {
                    let mut name = fuzz_text(&mut rng, 8);
                    if name.is_empty() {
                        name.push('c');
                    }
                    (name, (rng.next() % 100000) as i32)
                })
                .collect();
            let col_refs: Vec<(&str, i32)> = cols.iter().map(|(n, o)| (n.as_str(), *o)).collect();
            let body = row_description_body(&col_refs);

            let mut framed = bytes::BytesMut::new();
            framed.extend_from_slice(b"T");
            framed.extend_from_slice(&((body.len() as i32 + 4).to_be_bytes()));
            framed.extend_from_slice(&body);
            match backend::Message::parse(&mut framed).expect("reference parses RowDescription") {
                Some(backend::Message::RowDescription(rd)) => {
                    let fields: Vec<(String, u32)> = rd
                        .fields()
                        .map(|f| Ok((f.name().to_string(), f.type_oid())))
                        .collect()
                        .expect("reference fields");
                    assert_eq!(fields.len(), cols.len(), "reference column count");
                    for (i, (name, oid)) in fields.iter().enumerate() {
                        assert_eq!(name, &cols[i].0, "reference name {i}");
                        assert_eq!(*oid, cols[i].1 as u32, "reference oid {i}");
                    }
                }
                _ => panic!("the reference parser did not see a RowDescription"),
            }

            program.push_str(&format!(
                "Let body{case} be {}.\n\
                 Let (names{case}, oids{case}) be pgParseRowDescription(body{case}).\n\
                 Repeat for nm in names{case}:\n\
                 \x20   Show \"<{{nm}}>\".\n\
                 Show oids{case}.\n",
                logos_bytes(&body)
            ));
            for (name, _) in &cols {
                expected.push(format!("<{name}>"));
            }
            let oid_list: Vec<String> = cols.iter().map(|(_, o)| o.to_string()).collect();
            expected.push(format!("[{}]", oid_list.join(", ")));
        }
        assert_all_tiers(&program, &expected.join("\n"));
    }
}

/// FUZZ: the ParameterDescription parser over random OID lists, 8 shapes.
#[test]
fn fuzz_param_description_parser() {
    let mut rng = Lcg(0x0B1D_0B1D_0B1D_0B1D);
    let mut program = String::from("## Main\n");
    let mut expected = Vec::new();
    for case in 0..8 {
        let n = rng.below(7);
        let oids: Vec<i32> = (0..n).map(|_| (rng.next() % 1000000) as i32).collect();
        let body = param_description_body(&oids);
        program.push_str(&format!(
            "Let body{case} be {}.\n\
             Show pgParseParamDescription(body{case}).\n",
            logos_bytes(&body)
        ));
        let list: Vec<String> = oids.iter().map(|o| o.to_string()).collect();
        expected.push(format!("[{}]", list.join(", ")));
    }
    assert_all_tiers(&program, &expected.join("\n"));
}

/// FUZZ: pgDecimalAt round-trips random big decimals (up to 38 int digits +
/// up to 20 fraction digits, signed) EXACTLY, digit for digit, 16 shapes.
#[test]
fn fuzz_decimal_at_round_trips_exact_text() {
    let mut rng = Lcg(0xDEC1_3A17_2026_0901);
    for _chunk in 0..2 {
        let mut cells: Vec<String> = Vec::new();
        for _case in 0..8 {
            let sign = if rng.below(2) == 0 { "" } else { "-" };
            let int_len = 1 + rng.below(38);
            let mut int_part = String::new();
            int_part.push((b'1' + rng.below(9) as u8) as char);
            for _ in 1..int_len {
                int_part.push((b'0' + rng.below(10) as u8) as char);
            }
            let frac_len = rng.below(21);
            let text = if frac_len == 0 {
                format!("{sign}{int_part}")
            } else {
                let mut frac = String::new();
                for _ in 0..frac_len - 1 {
                    frac.push((b'0' + rng.below(10) as u8) as char);
                }
                frac.push((b'1' + rng.below(9) as u8) as char);
                format!("{sign}{int_part}.{frac}")
            };
            cells.push(text);
        }
        let cell_rows: Vec<Vec<Option<&str>>> =
            cells.iter().map(|c| vec![Some(c.as_str())]).collect();
        let row_refs: Vec<&[Option<&str>]> = cell_rows.iter().map(|r| r.as_slice()).collect();
        let preamble = qr_preamble(&[("x", 1700)], &row_refs, "SELECT 8");
        let mut program = format!("## Main\n{preamble}");
        for (i, _) in cells.iter().enumerate() {
            program.push_str(&format!(
                "Let d{i} be pgDecimalAt(qr, {}, 1).\n\
                 Inspect d{i}:\n\
                 \x20   When Ok (v):\n\
                 \x20       Show v.\n\
                 \x20   When Err (e):\n\
                 \x20       Show e.\n",
                i + 1
            ));
        }
        assert_all_tiers(&program, &cells.join("\n"));
    }
}

/// FUZZ: pgUuidAt round-trips random UUIDs (random 16 bytes rendered
/// canonically, upper- and lowercase inputs) through parse + format, 12
/// shapes.
#[test]
fn fuzz_uuid_at_round_trips() {
    let mut rng = Lcg(0x0071_D5EE_D000_2950);
    for _chunk in 0..2 {
        let mut inputs: Vec<String> = Vec::new();
        let mut canonical: Vec<String> = Vec::new();
        for _case in 0..6 {
            let bytes: Vec<u8> = (0..16).map(|_| (rng.next() % 256) as u8).collect();
            let h: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            let canon = format!(
                "{}-{}-{}-{}-{}",
                &h[0..8],
                &h[8..12],
                &h[12..16],
                &h[16..20],
                &h[20..32]
            );
            let input = if rng.below(2) == 0 {
                canon.clone()
            } else {
                canon.to_uppercase()
            };
            inputs.push(input);
            canonical.push(canon);
        }
        let cell_rows: Vec<Vec<Option<&str>>> =
            inputs.iter().map(|c| vec![Some(c.as_str())]).collect();
        let row_refs: Vec<&[Option<&str>]> = cell_rows.iter().map(|r| r.as_slice()).collect();
        let preamble = qr_preamble(&[("u", 2950)], &row_refs, "SELECT 6");
        let mut program = format!("## Main\n{preamble}");
        for (i, _) in inputs.iter().enumerate() {
            program.push_str(&format!(
                "Let u{i} be pgUuidAt(qr, {}, 1).\n\
                 Inspect u{i}:\n\
                 \x20   When Ok (v):\n\
                 \x20       Show uuidFormat(v).\n\
                 \x20   When Err (e):\n\
                 \x20       Show e.\n",
                i + 1
            ));
        }
        assert_all_tiers(&program, &canonical.join("\n"));
    }
}
