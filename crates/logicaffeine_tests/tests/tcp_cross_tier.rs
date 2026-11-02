//! ════════════════════════════════════════════════════════════════════════════════════════════
//! A1 — CROSS-TIER TCP LOCK: the tcp.lg client natives run byte-identically on the BYTECODE VM
//! and the TREE-WALKER. Both tiers compile the SAME program against the SAME kind of scripted
//! loopback peer; the VM path is `run_vm_io_async` (opcodes `Op::Tcp*` → `VmBlock::Tcp*` → the
//! async io driver); the tree-walker path is `interpret_for_ui`.
//!
//!  ⚠️  YOU DO NOT GET TO WEAKEN THIS LOCK TO MAKE IT PASS.  ⚠️
//!  A divergence here means TCP drifted between tiers — fix the TIER (`run_vm_io_async` / the
//!  VM tcp opcodes / the shared `TcpRegistry` semantics), never relax the `assert_eq!`.
//!  Strictly monotone: add shapes, never remove them.
//! ════════════════════════════════════════════════════════════════════════════════════════════

use std::time::Duration;

use logicaffeine_compile::{interpret_for_ui, run_vm_io_async};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Run `program` on one tier — the VM io driver or the tree-walker — under a hang guard.
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
    tokio::time::timeout(Duration::from_secs(10), fut)
        .await
        .expect("the program must not hang")
}

/// A scripted peer: accepts ONE connection, echoes until close.
fn spawn_echo() -> (u16, tokio::task::JoinHandle<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("binds");
    listener.set_nonblocking(true).expect("nonblocking");
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        let listener = TcpListener::from_std(listener).expect("tokio listener");
        if let Ok((mut s, _)) = listener.accept().await {
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
        }
    });
    (port, handle)
}

fn round_trip_program(port: u16) -> String {
    format!(
        "## Main\n\
         Let c be tcpConnect(\"127.0.0.1\", {port}).\n\
         Inspect c:\n\
         \x20   When Ok (conn):\n\
         \x20       Let sent be tcpSend(conn, text_bytes(\"parity\")).\n\
         \x20       Inspect sent:\n\
         \x20           When Ok (n):\n\
         \x20               Show n.\n\
         \x20               Let got be tcpRecvExact(conn, 6).\n\
         \x20               Inspect got:\n\
         \x20                   When Ok (data):\n\
         \x20                       Show text_from_bytes(data).\n\
         \x20                       Show data.\n\
         \x20                   When Err (e):\n\
         \x20                       Show e.\n\
         \x20           When Err (e):\n\
         \x20               Show e.\n\
         \x20       Let closed be tcpClose(conn).\n\
         \x20       Show \"done\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n"
    )
}

/// The echo round-trip produces the SAME lines on both tiers (each against its
/// own fresh scripted peer — sockets are per-run, semantics must be identical).
#[tokio::test]
async fn tcp_round_trip_agrees_across_tiers() {
    let (tw_port, _tw_srv) = spawn_echo();
    let tw = run_tier(&round_trip_program(tw_port), false).await;
    assert!(tw.error.is_none(), "tree-walker errored: {:?}", tw.error);

    let (vm_port, _vm_srv) = spawn_echo();
    let vm = run_tier(&round_trip_program(vm_port), true).await;
    assert!(vm.error.is_none(), "vm errored: {:?}", vm.error);

    assert_eq!(
        tw.lines, vm.lines,
        "TIER DIVERGENCE: tcp round-trip output differs"
    );
    assert_eq!(
        tw.lines,
        vec!["6", "parity", "[112, 97, 114, 105, 116, 121]", "done"]
    );
}

/// Error shapes agree too: a refused connect is the same `Err`-arm outcome on both tiers.
#[tokio::test]
async fn tcp_refused_connect_agrees_across_tiers() {
    let program = "## Main\n\
                   Let c be tcpConnect(\"127.0.0.1\", 1).\n\
                   Inspect c:\n\
                   \x20   When Ok (conn):\n\
                   \x20       Show \"connected\".\n\
                   \x20   When Err (e):\n\
                   \x20       Show \"refused\".\n";
    let tw = run_tier(program, false).await;
    let vm = run_tier(program, true).await;
    assert!(
        tw.error.is_none() && vm.error.is_none(),
        "tw={:?} vm={:?}",
        tw.error,
        vm.error
    );
    assert_eq!(tw.lines, vm.lines, "TIER DIVERGENCE on refused connect");
    assert_eq!(tw.lines, vec!["refused"]);
}

/// The one-program listen→connect→accept→talk shape (passive open through the
/// kernel backlog) produces the SAME lines on both tiers.
#[tokio::test]
async fn tcp_listen_accept_agrees_across_tiers() {
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
        \x20                               Let s1 be tcpSend(client, text_bytes(\"xt\")).\n\
        \x20                               Let r1 be tcpRecvExact(server, 2).\n\
        \x20                               Inspect r1:\n\
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
    let tw = run_tier(program, false).await;
    assert!(tw.error.is_none(), "tree-walker errored: {:?}", tw.error);
    let vm = run_tier(program, true).await;
    assert!(vm.error.is_none(), "vm errored: {:?}", vm.error);
    assert_eq!(tw.lines, vm.lines, "TIER DIVERGENCE on listen/accept");
    assert_eq!(tw.lines, vec!["xt"]);
}

/// A read timeout fires the same `Err`-arm outcome on both tiers.
#[tokio::test]
async fn tcp_timeout_agrees_across_tiers() {
    async fn one_tier(use_vm: bool) -> Vec<String> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("binds");
        listener.set_nonblocking(true).expect("nonblocking");
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let listener = TcpListener::from_std(listener).expect("tokio listener");
            if let Ok((s, _)) = listener.accept().await {
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
             \x20       Let got be tcpRecv(conn, 8).\n\
             \x20       Inspect got:\n\
             \x20           When Ok (data):\n\
             \x20               Show data.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20   When Err (e):\n\
             \x20       Show e.\n"
        );
        let r = run_tier(&program, use_vm).await;
        assert!(r.error.is_none(), "tier vm={use_vm} errored: {:?}", r.error);
        r.lines
    }
    let tw = one_tier(false).await;
    let vm = one_tier(true).await;
    assert_eq!(tw, vm, "TIER DIVERGENCE on timeout");
    assert_eq!(tw, vec!["tcp: operation timed out"]);
}

/// Early-EOF on `tcpRecvExact` is the same `Err`-arm outcome on both tiers.
#[tokio::test]
async fn tcp_torn_frame_agrees_across_tiers() {
    async fn one_tier(use_vm: bool) -> Vec<String> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("binds");
        listener.set_nonblocking(true).expect("nonblocking");
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let listener = TcpListener::from_std(listener).expect("tokio listener");
            if let Ok((mut s, _)) = listener.accept().await {
                let _ = s.write_all(b"ab").await;
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
        let r = run_tier(&program, use_vm).await;
        assert!(r.error.is_none(), "tier vm={use_vm} errored: {:?}", r.error);
        r.lines
    }
    let tw = one_tier(false).await;
    let vm = one_tier(true).await;
    assert_eq!(tw, vm, "TIER DIVERGENCE on torn frame");
    assert_eq!(tw, vec!["torn"]);
}
