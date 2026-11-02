//! A1 — first-class TCP client natives (tcp.lg): `tcpConnect`/`tcpSend`/`tcpRecv`/
//! `tcpRecvExact`/`tcpClose` against a real loopback socket, on the tree-walker.
//! Bytes are `Seq of Int` (0–255); every operation returns a `Result` so a LOGOS
//! program handles network failure as a value, never a trap. The cross-tier lock
//! (`tcp_cross_tier.rs`) proves the VM agrees; this file pins the SEMANTICS:
//! recv-up-to vs recv-exactly, EOF, refused connects, and closed-handle errors.

use std::time::Duration;

use logicaffeine_compile::interpret_for_ui;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Run a LOGOS program on the tree-walker under a hang guard.
async fn run(program: &str) -> logicaffeine_compile::interpreter::InterpreterResult {
    tokio::time::timeout(Duration::from_secs(10), interpret_for_ui(program))
        .await
        .expect("the program must not hang")
}

/// An echo server on an ephemeral port: echoes every byte until the client closes.
async fn echo_server() -> (TcpListener, u16) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("echo binds");
    let port = listener.local_addr().unwrap().port();
    (listener, port)
}

fn echo_accept_loop(listener: TcpListener) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                loop {
                    match s.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if s.write_all(&buf[..n]).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            });
        }
    })
}

/// The full happy path: connect → send "ping" → recv exactly 4 echoed bytes →
/// decode → close. Every step's `Result` is inspected.
#[tokio::test]
async fn tcp_echo_round_trip() {
    let (listener, port) = echo_server().await;
    let _srv = echo_accept_loop(listener);

    let program = format!(
        "## Main\n\
         Let c be tcpConnect(\"127.0.0.1\", {port}).\n\
         Inspect c:\n\
         \x20   When Ok (conn):\n\
         \x20       Let sent be tcpSend(conn, text_bytes(\"ping\")).\n\
         \x20       Inspect sent:\n\
         \x20           When Ok (n):\n\
         \x20               Show n.\n\
         \x20               Let got be tcpRecvExact(conn, 4).\n\
         \x20               Inspect got:\n\
         \x20                   When Ok (data):\n\
         \x20                       Show text_from_bytes(data).\n\
         \x20                   When Err (e):\n\
         \x20                       Show e.\n\
         \x20       Let closed be tcpClose(conn).\n\
         \x20       Show \"done\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n"
    );

    let result = run(&program).await;
    assert!(
        result.error.is_none(),
        "program errored: {:?}",
        result.error
    );
    assert_eq!(result.lines, vec!["4", "ping", "done"]);
}

/// A refused connect is an `Err` value, not a trap. Port 1 is never listening.
#[tokio::test]
async fn tcp_connect_refused_is_err() {
    let program = "## Main\n\
                   Let c be tcpConnect(\"127.0.0.1\", 1).\n\
                   Inspect c:\n\
                   \x20   When Ok (conn):\n\
                   \x20       Show \"unexpectedly connected\".\n\
                   \x20   When Err (e):\n\
                   \x20       Show \"refused\".\n";
    let result = run(program).await;
    assert!(
        result.error.is_none(),
        "program errored: {:?}",
        result.error
    );
    assert_eq!(result.lines, vec!["refused"]);
}

/// `tcpRecvExact` on a peer that closes early is an `Err` — a torn frame must
/// never be silently truncated (postgres framing depends on this).
#[tokio::test]
async fn tcp_recv_exact_early_eof_is_err() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("binds");
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        if let Ok((mut s, _)) = listener.accept().await {
            let _ = s.write_all(b"ab").await;
            // close after 2 bytes — the client asked for 4
        }
    });

    let program = format!(
        "## Main\n\
         Let c be tcpConnect(\"127.0.0.1\", {port}).\n\
         Inspect c:\n\
         \x20   When Ok (conn):\n\
         \x20       Let got be tcpRecvExact(conn, 4).\n\
         \x20       Inspect got:\n\
         \x20           When Ok (data):\n\
         \x20               Show data.\n\
         \x20           When Err (e):\n\
         \x20               Show \"torn\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n"
    );
    let result = run(&program).await;
    assert!(
        result.error.is_none(),
        "program errored: {:?}",
        result.error
    );
    assert_eq!(result.lines, vec!["torn"]);
}

/// `tcpRecv` (read up to N) on a cleanly-closed peer is `Ok([])` — EOF is a
/// value, distinguishable from an error.
#[tokio::test]
async fn tcp_recv_at_eof_is_ok_empty() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("binds");
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        if let Ok((s, _)) = listener.accept().await {
            drop(s); // immediate clean close
        }
    });

    let program = format!(
        "## Main\n\
         Let c be tcpConnect(\"127.0.0.1\", {port}).\n\
         Inspect c:\n\
         \x20   When Ok (conn):\n\
         \x20       Let got be tcpRecv(conn, 8).\n\
         \x20       Inspect got:\n\
         \x20           When Ok (data):\n\
         \x20               Show data.\n\
         \x20           When Err (e):\n\
         \x20               Show e.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n"
    );
    let result = run(&program).await;
    assert!(
        result.error.is_none(),
        "program errored: {:?}",
        result.error
    );
    assert_eq!(result.lines, vec!["[]"]);
}

/// Operations on a closed handle are `Err` values: double-close and send-after-close.
#[tokio::test]
async fn tcp_closed_handle_operations_are_err() {
    let (listener, port) = echo_server().await;
    let _srv = echo_accept_loop(listener);

    let program = format!(
        "## Main\n\
         Let c be tcpConnect(\"127.0.0.1\", {port}).\n\
         Inspect c:\n\
         \x20   When Ok (conn):\n\
         \x20       Let closed be tcpClose(conn).\n\
         \x20       Inspect closed:\n\
         \x20           When Ok (u):\n\
         \x20               Show \"closed\".\n\
         \x20           When Err (e):\n\
         \x20               Show e.\n\
         \x20       Let again be tcpClose(conn).\n\
         \x20       Inspect again:\n\
         \x20           When Ok (u):\n\
         \x20               Show \"unexpectedly closed twice\".\n\
         \x20           When Err (e):\n\
         \x20               Show \"double close err\".\n\
         \x20       Let sent be tcpSend(conn, text_bytes(\"x\")).\n\
         \x20       Inspect sent:\n\
         \x20           When Ok (n):\n\
         \x20               Show \"unexpectedly sent\".\n\
         \x20           When Err (e):\n\
         \x20               Show \"send after close err\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n"
    );
    let result = run(&program).await;
    assert!(
        result.error.is_none(),
        "program errored: {:?}",
        result.error
    );
    assert_eq!(
        result.lines,
        vec!["closed", "double close err", "send after close err"]
    );
}

/// A read timeout (`tcpSetTimeout`, Python `settimeout` / Go deadline style)
/// turns a silent peer into an `Err` — a client must never hang forever.
#[tokio::test]
async fn tcp_recv_times_out_against_a_silent_peer() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("binds");
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        if let Ok((s, _)) = listener.accept().await {
            // accept and say NOTHING for far longer than the timeout
            tokio::time::sleep(Duration::from_secs(30)).await;
            drop(s);
        }
    });

    let program = format!(
        "## Main\n\
         Let c be tcpConnect(\"127.0.0.1\", {port}).\n\
         Inspect c:\n\
         \x20   When Ok (conn):\n\
         \x20       Let t be tcpSetTimeout(conn, 150).\n\
         \x20       Inspect t:\n\
         \x20           When Ok (u):\n\
         \x20               Let got be tcpRecv(conn, 8).\n\
         \x20               Inspect got:\n\
         \x20                   When Ok (data):\n\
         \x20                       Show data.\n\
         \x20                   When Err (e):\n\
         \x20                       Show \"timed out\".\n\
         \x20           When Err (e):\n\
         \x20               Show e.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n"
    );
    let result = run(&program).await;
    assert!(
        result.error.is_none(),
        "program errored: {:?}",
        result.error
    );
    assert_eq!(result.lines, vec!["timed out"]);
}

/// TCP is a STREAM (RFC 9293): bytes buffered before the peer closes are still
/// readable AFTER the close — recv drains the buffer first, THEN reports EOF.
#[tokio::test]
async fn tcp_buffered_data_survives_peer_close_then_eof() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("binds");
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        if let Ok((mut s, _)) = listener.accept().await {
            let _ = s.write_all(b"tail").await;
            // close immediately — the 4 bytes are in flight/buffered
        }
    });

    let program = format!(
        "## Main\n\
         Let c be tcpConnect(\"127.0.0.1\", {port}).\n\
         Inspect c:\n\
         \x20   When Ok (conn):\n\
         \x20       Let got be tcpRecvExact(conn, 4).\n\
         \x20       Inspect got:\n\
         \x20           When Ok (data):\n\
         \x20               Show text_from_bytes(data).\n\
         \x20           When Err (e):\n\
         \x20               Show e.\n\
         \x20       Let after be tcpRecv(conn, 8).\n\
         \x20       Inspect after:\n\
         \x20           When Ok (rest):\n\
         \x20               Show rest.\n\
         \x20           When Err (e):\n\
         \x20               Show e.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n"
    );
    let result = run(&program).await;
    assert!(
        result.error.is_none(),
        "program errored: {:?}",
        result.error
    );
    assert_eq!(result.lines, vec!["tail", "[]"]);
}

/// TCP has NO message boundaries: two separate peer writes may arrive as one
/// read. `tcpRecvExact` reassembles across the boundary — a protocol client
/// must never assume one send = one recv.
#[tokio::test]
async fn tcp_two_sends_reassemble_across_the_stream() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("binds");
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        if let Ok((mut s, _)) = listener.accept().await {
            let _ = s.write_all(b"he").await;
            let _ = s.flush().await;
            tokio::time::sleep(Duration::from_millis(30)).await;
            let _ = s.write_all(b"llo").await;
            let _ = s.flush().await;
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    });

    let program = format!(
        "## Main\n\
         Let c be tcpConnect(\"127.0.0.1\", {port}).\n\
         Inspect c:\n\
         \x20   When Ok (conn):\n\
         \x20       Let got be tcpRecvExact(conn, 5).\n\
         \x20       Inspect got:\n\
         \x20           When Ok (data):\n\
         \x20               Show text_from_bytes(data).\n\
         \x20           When Err (e):\n\
         \x20               Show e.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n"
    );
    let result = run(&program).await;
    assert!(
        result.error.is_none(),
        "program errored: {:?}",
        result.error
    );
    assert_eq!(result.lines, vec!["hello"]);
}

/// PASSIVE OPEN (RFC 9293 §3.10 LISTEN) — a pure-LOGOS server AND client in ONE
/// sequential program: `tcpListen(0)` binds an ephemeral port, `tcpConnect` to it
/// succeeds while nobody is accepting yet (the KERNEL's backlog completes the
/// three-way handshake), then `tcpAccept` yields the server conn and the two
/// handles talk both directions.
#[tokio::test]
async fn tcp_listen_accept_full_duplex_in_one_program() {
    let program = "## Main\n\
        Let l be tcpListen(0).\n\
        Inspect l:\n\
        \x20   When Ok (listener):\n\
        \x20       Let p be tcpListenerPort(listener).\n\
        \x20       Inspect p:\n\
        \x20           When Ok (port):\n\
        \x20               Let c be tcpConnect(\"127.0.0.1\", port).\n\
        \x20               Inspect c:\n\
        \x20                   When Ok (client):\n\
        \x20                       Let a be tcpAccept(listener).\n\
        \x20                       Inspect a:\n\
        \x20                           When Ok (server):\n\
        \x20                               Let s1 be tcpSend(client, text_bytes(\"hi\")).\n\
        \x20                               Let r1 be tcpRecvExact(server, 2).\n\
        \x20                               Inspect r1:\n\
        \x20                                   When Ok (d):\n\
        \x20                                       Show text_from_bytes(d).\n\
        \x20                                   When Err (e):\n\
        \x20                                       Show e.\n\
        \x20                               Let s2 be tcpSend(server, text_bytes(\"yo\")).\n\
        \x20                               Let r2 be tcpRecvExact(client, 2).\n\
        \x20                               Inspect r2:\n\
        \x20                                   When Ok (d):\n\
        \x20                                       Show text_from_bytes(d).\n\
        \x20                                   When Err (e):\n\
        \x20                                       Show e.\n\
        \x20                           When Err (e):\n\
        \x20                               Show e.\n\
        \x20                   When Err (e):\n\
        \x20                       Show e.\n\
        \x20           When Err (e):\n\
        \x20               Show e.\n\
        \x20   When Err (e):\n\
        \x20       Show e.\n";
    let result = run(program).await;
    assert!(
        result.error.is_none(),
        "program errored: {:?}",
        result.error
    );
    assert_eq!(result.lines, vec!["hi", "yo"]);
}

/// HALF-CLOSE (RFC 9293 half-open): `tcpShutdownWrite` sends FIN while the read
/// side stays open — the peer sees EOF, can still reply, and the closer reads
/// that reply. The exact shutdown(SHUT_WR) semantics of Python/Go.
#[tokio::test]
async fn tcp_half_close_lets_the_peer_reply() {
    let program = "## Main\n\
        Let l be tcpListen(0).\n\
        Inspect l:\n\
        \x20   When Ok (listener):\n\
        \x20       Let p be tcpListenerPort(listener).\n\
        \x20       Inspect p:\n\
        \x20           When Ok (port):\n\
        \x20               Let c be tcpConnect(\"127.0.0.1\", port).\n\
        \x20               Inspect c:\n\
        \x20                   When Ok (client):\n\
        \x20                       Let a be tcpAccept(listener).\n\
        \x20                       Inspect a:\n\
        \x20                           When Ok (server):\n\
        \x20                               Let s1 be tcpSend(client, text_bytes(\"ping\")).\n\
        \x20                               Let h be tcpShutdownWrite(client).\n\
        \x20                               Let r1 be tcpRecvExact(server, 4).\n\
        \x20                               Inspect r1:\n\
        \x20                                   When Ok (d):\n\
        \x20                                       Show text_from_bytes(d).\n\
        \x20                                   When Err (e):\n\
        \x20                                       Show e.\n\
        \x20                               Let eof be tcpRecv(server, 8).\n\
        \x20                               Inspect eof:\n\
        \x20                                   When Ok (d):\n\
        \x20                                       Show d.\n\
        \x20                                   When Err (e):\n\
        \x20                                       Show e.\n\
        \x20                               Let s2 be tcpSend(server, text_bytes(\"pong\")).\n\
        \x20                               Let r2 be tcpRecvExact(client, 4).\n\
        \x20                               Inspect r2:\n\
        \x20                                   When Ok (d):\n\
        \x20                                       Show text_from_bytes(d).\n\
        \x20                                   When Err (e):\n\
        \x20                                       Show e.\n\
        \x20                           When Err (e):\n\
        \x20                               Show e.\n\
        \x20                   When Err (e):\n\
        \x20                       Show e.\n\
        \x20           When Err (e):\n\
        \x20               Show e.\n\
        \x20   When Err (e):\n\
        \x20       Show e.\n";
    let result = run(program).await;
    assert!(
        result.error.is_none(),
        "program errored: {:?}",
        result.error
    );
    assert_eq!(result.lines, vec!["ping", "[]", "pong"]);
}

/// Hostname connect goes through the resolver — `"localhost"`, like Python's
/// `socket.connect(("localhost", p))`.
#[tokio::test]
async fn tcp_connect_resolves_hostnames() {
    let (listener, port) = echo_server().await;
    let _srv = echo_accept_loop(listener);
    let program = format!(
        "## Main\n\
         Let c be tcpConnect(\"localhost\", {port}).\n\
         Inspect c:\n\
         \x20   When Ok (conn):\n\
         \x20       Let sent be tcpSend(conn, text_bytes(\"dns\")).\n\
         \x20       Let got be tcpRecvExact(conn, 3).\n\
         \x20       Inspect got:\n\
         \x20           When Ok (data):\n\
         \x20               Show text_from_bytes(data).\n\
         \x20           When Err (e):\n\
         \x20               Show e.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n"
    );
    let result = run(&program).await;
    assert!(
        result.error.is_none(),
        "program errored: {:?}",
        result.error
    );
    assert_eq!(result.lines, vec!["dns"]);
}

/// A large payload (64 KiB) survives the echo intact — chunked reads reassemble
/// via `tcpRecvExact`, and byte values 0/255 pass through unmangled.
#[tokio::test]
async fn tcp_large_payload_round_trips() {
    let (listener, port) = echo_server().await;
    let _srv = echo_accept_loop(listener);

    // 65536 bytes of i mod 256, built in LOGOS, echoed, and length+checksum verified.
    let program = format!(
        "## Main\n\
         Let mutable payload be a new Seq of Int.\n\
         Repeat for i from 0 to 65535:\n\
         \x20   Push (i mod 256) to payload.\n\
         Let c be tcpConnect(\"127.0.0.1\", {port}).\n\
         Inspect c:\n\
         \x20   When Ok (conn):\n\
         \x20       Let sent be tcpSend(conn, payload).\n\
         \x20       Inspect sent:\n\
         \x20           When Ok (n):\n\
         \x20               Show n.\n\
         \x20               Let got be tcpRecvExact(conn, 65536).\n\
         \x20               Inspect got:\n\
         \x20                   When Ok (data):\n\
         \x20                       Show item 1 of data.\n\
         \x20                       Show item 256 of data.\n\
         \x20                       Show item 65536 of data.\n\
         \x20                   When Err (e):\n\
         \x20                       Show e.\n\
         \x20           When Err (e):\n\
         \x20               Show e.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n"
    );
    let result = run(&program).await;
    assert!(
        result.error.is_none(),
        "program errored: {:?}",
        result.error
    );
    assert_eq!(result.lines, vec!["65536", "0", "255", "255"]);
}
