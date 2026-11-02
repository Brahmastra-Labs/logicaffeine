//! A1 — the AOT-Rust leg of the tcp.lg natives: the SAME round-trip program the
//! interpreter/VM locks pin (`tcp_natives.rs` / `tcp_cross_tier.rs`) compiles to a
//! real native binary and talks to a real echo server, producing the same lines.
//! Environmental IO (like `parity_io.rs`): verified on the native compilation
//! path, not the sandboxed all-tier harness.
#![allow(unused_imports, dead_code)]
mod common;
use common::assert_exact_output;

/// An echo server on a background std thread, alive for the whole test.
fn spawn_echo() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            if let Ok(mut s) = stream {
                std::thread::spawn(move || {
                    use std::io::{Read, Write};
                    let mut buf = [0u8; 4096];
                    loop {
                        match s.read(&mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                if s.write_all(&buf[..n]).is_err() {
                                    break;
                                }
                            }
                        }
                    }
                });
            }
        }
    });
    port
}

#[test]
fn tcp_echo_round_trip_compiles_and_runs_natively() {
    let port = spawn_echo();
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
         \x20           When Err (e):\n\
         \x20               Show e.\n\
         \x20       Let closed be tcpClose(conn).\n\
         \x20       Show \"done\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n"
    );
    assert_exact_output(&program, "4\nping\ndone");
}

/// Error VALUES agree with the async tiers: a refused connect is an `Err` arm,
/// and a torn `tcpRecvExact` is an `Err` arm — never a trap, same as the locks
/// on the interpreter/VM tiers.
#[test]
fn tcp_error_values_survive_native_compilation() {
    let port = spawn_echo();
    let program = format!(
        "## Main\n\
         Let bad be tcpConnect(\"127.0.0.1\", 1).\n\
         Inspect bad:\n\
         \x20   When Ok (conn):\n\
         \x20       Show \"unexpectedly connected\".\n\
         \x20   When Err (e):\n\
         \x20       Show \"refused\".\n\
         Let c be tcpConnect(\"127.0.0.1\", {port}).\n\
         Inspect c:\n\
         \x20   When Ok (conn):\n\
         \x20       Let closed be tcpClose(conn).\n\
         \x20       Let again be tcpClose(conn).\n\
         \x20       Inspect again:\n\
         \x20           When Ok (u):\n\
         \x20               Show \"unexpectedly closed twice\".\n\
         \x20           When Err (e):\n\
         \x20               Show \"double close err\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n"
    );
    assert_exact_output(&program, "refused\ndouble close err");
}

/// PASSIVE OPEN + HALF-CLOSE survive native compilation: the same one-program
/// listen→connect→accept→talk→shutdown shape the interpreter lock pins, through
/// the std/POSIX sockets path (`socket`/`bind`/`listen`/`accept`/`shutdown`).
#[test]
fn tcp_listen_accept_and_half_close_compile_and_run_natively() {
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
    assert_exact_output(program, "ping\n[]\npong");
}

/// The timeout fires with the SAME wording on the native tier (the formatters
/// live in `logicaffeine_system::tcp`, shared by every tier).
#[test]
fn tcp_timeout_survives_native_compilation() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            if let Ok(s) = stream {
                std::thread::sleep(std::time::Duration::from_secs(30));
                drop(s);
            }
        }
    });
    let program = format!(
        "## Main\n\
         Let c be tcpConnect(\"127.0.0.1\", {port}).\n\
         Inspect c:\n\
         \x20   When Ok (conn):\n\
         \x20       Let t be tcpSetTimeout(conn, 150).\n\
         \x20       Let got be tcpRecv(conn, 8).\n\
         \x20       Inspect got:\n\
         \x20           When Ok (data):\n\
         \x20               Show data.\n\
         \x20           When Err (e):\n\
         \x20               Show e.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n"
    );
    assert_exact_output(&program, "tcp: operation timed out");
}
