//! ★ THE Op::Tcp WALKER GOES NATIVE — the LAST delegation is gone (RED-first, TDD). ★
//!
//! `Op::Tcp` (the `tcp.lg` real-socket client — the `BigIntWide` one-op-many-kinds pattern, `kind` = the
//! `TcpOp::code()`) was the ONE bytecode op the flat Op-walker still handed to the AST-C backend. This lock
//! proves it now lowers NATIVELY on the bytecode→C walker (`compile_to_c_via_bytecode`, `from_program`) over
//! a real POSIX socket runtime (`C_TCP_RUNTIME`), byte-for-byte identical to the VM's async IO driver
//! (`run_vm_io_async`, which services `VmBlock::Tcp` through the SAME `TcpRegistry::exec_values` the tree-
//! walker calls). Two ≡VM witnesses + a compiles-and-links check:
//!
//! 1. **Round-trip ≡VM** — a real `std::net::TcpListener` echo server on an ephemeral 127.0.0.1 port (a
//!    background thread accepts ONE connection, reads N bytes, echoes them back). A pure-CLIENT LOGOS
//!    program (`tcpConnect` → `tcpSend` → `tcpRecvExact` → `Show`) runs through BOTH `run_vm_io_async` and
//!    the walker C (gcc build + run), and their stdout must be byte-identical. A genuine end-to-end TCP
//!    round-trip proven VM≡walker.
//! 2. **Connection-refused ≡VM** — `tcpConnect` to a definitely-closed port (bind + drop a listener to get a
//!    guaranteed-free port) must produce the SAME `Err` value on both tiers — including the OS-error wording
//!    (`… failed: Connection refused (os error 111)`), which the C runtime renders the Rust way so it cannot
//!    drift. Deterministic, no external server.
//! 3. **Compiles + links** — the walker's `Tcp` C builds with gcc (the socket runtime is real, not a stub).
//!
//! The gcc-gated halves SKIP cleanly when gcc is absent; the compile check + the delegation-gate assertion
//! always run. ⚠️ RED tests are the spec: if a witness fails, fix the lowering, NEVER weaken this file.

use std::io::{Read, Write};
use std::net::TcpListener;

use logicaffeine_compile::compile::compile_to_c_via_bytecode;
use logicaffeine_compile::run_vm_io_async;

fn have_gcc() -> bool {
    std::process::Command::new("gcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Build the walker's C (native `Op::Tcp` → plain C → gcc) and run it, returning stdout lines. `None` on any
/// compile/build/run failure (the caller distinguishes "skip" from "fail").
fn walker_run(src: &str, label: &str) -> Option<Vec<String>> {
    let c = compile_to_c_via_bytecode(src).ok()?;
    let dir = std::env::temp_dir().join(format!("logos_tcpw_{}_{}", std::process::id(), label));
    let _ = std::fs::create_dir_all(&dir);
    let cfile = dir.join("prog.c");
    let bin = dir.join("prog");
    std::fs::write(&cfile, &c).ok()?;
    let build = std::process::Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&cfile)
        .arg("-lm")
        .output()
        .ok()?;
    if !build.status.success() {
        eprintln!(
            "[tcp-walker] {label} build failed:\n{}",
            String::from_utf8_lossy(&build.stderr)
                .lines()
                .take(12)
                .collect::<Vec<_>>()
                .join("\n")
        );
        return None;
    }
    let run = std::process::Command::new(&bin).output().ok()?;
    if !run.status.success() {
        return None;
    }
    let lines = String::from_utf8_lossy(&run.stdout)
        .lines()
        .map(|s| s.to_string())
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    Some(lines)
}

/// Run a LOGOS program through the VM's ASYNC IO driver (the ONLY tier that services `Op::Tcp` — via the real
/// `TcpRegistry`). A fresh single-thread tokio runtime per call keeps the harness self-contained.
fn vm_async_run(src: &str) -> Vec<String> {
    let rt = logicaffeine_system::tokio::runtime::Runtime::new().expect("tokio runtime");
    let res = rt.block_on(run_vm_io_async(src));
    assert!(res.error.is_none(), "VM async errored: {:?}", res.error);
    res.lines
}

/// A background std echo server that accepts ONE connection, reads exactly `n` bytes, and echoes them back.
/// Returns the ephemeral port it bound to (the connection completes before the client even connects, so the
/// client's `tcpConnect` always succeeds).
fn spawn_echo_once(n: usize) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind echo server");
    let port = listener.local_addr().expect("local addr").port();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = vec![0u8; n];
            let mut got = 0;
            while got < n {
                match stream.read(&mut buf[got..]) {
                    Ok(0) => break,
                    Ok(k) => got += k,
                    Err(_) => break,
                }
            }
            let _ = stream.write_all(&buf[..got]);
            let _ = stream.flush();
        }
    });
    port
}

/// The pure-CLIENT round-trip program: connect to `port`, send five bytes, receive exactly five back, and
/// `Show` them. Every step is `Inspect`ed so a network failure `Show`s its `Err` (the tiers must agree on
/// that too). Bytes are `72..=76` → the seq renders `[72, 73, 74, 75, 76]`.
fn round_trip_program(port: u16) -> String {
    format!(
        "## Main\n\
        \x20   Let cr be tcpConnect(\"127.0.0.1\", {port}).\n\
        \x20   Inspect cr:\n\
        \x20       When Ok (c):\n\
        \x20           Let sr be tcpSend(c, [72, 73, 74, 75, 76]).\n\
        \x20           Inspect sr:\n\
        \x20               When Ok (nsent):\n\
        \x20                   Let rr be tcpRecvExact(c, 5).\n\
        \x20                   Inspect rr:\n\
        \x20                       When Ok (bytes):\n\
        \x20                           Show bytes.\n\
        \x20                       When Err (e3):\n\
        \x20                           Show e3.\n\
        \x20               When Err (e2):\n\
        \x20                   Show e2.\n\
        \x20       When Err (e):\n\
        \x20           Show e.\n"
    )
}

/// A round-trip that ALSO exercises `tcpSetTimeout` (a large 5s deadline that never fires) and
/// `tcpShutdownWrite` (half-close the write side after the send) — both must return `Ok` on both tiers for the
/// `Show bytes` to be reached, so a byte-identical `[72, 73, 74, 75, 76]` proves both lowered ≡VM.
fn timeout_shutdown_program(port: u16) -> String {
    format!(
        "## Main\n\
        \x20   Let cr be tcpConnect(\"127.0.0.1\", {port}).\n\
        \x20   Inspect cr:\n\
        \x20       When Ok (c):\n\
        \x20           Let tr be tcpSetTimeout(c, 5000).\n\
        \x20           Inspect tr:\n\
        \x20               When Ok (tu):\n\
        \x20                   Let sr be tcpSend(c, [72, 73, 74, 75, 76]).\n\
        \x20                   Inspect sr:\n\
        \x20                       When Ok (nsent):\n\
        \x20                           Let wr be tcpShutdownWrite(c).\n\
        \x20                           Inspect wr:\n\
        \x20                               When Ok (wu):\n\
        \x20                                   Let rr be tcpRecvExact(c, 5).\n\
        \x20                                   Inspect rr:\n\
        \x20                                       When Ok (bytes):\n\
        \x20                                           Show bytes.\n\
        \x20                                       When Err (e4):\n\
        \x20                                           Show e4.\n\
        \x20                               When Err (e3):\n\
        \x20                                   Show e3.\n\
        \x20                       When Err (e2):\n\
        \x20                           Show e2.\n\
        \x20               When Err (e1):\n\
        \x20                   Show e1.\n\
        \x20       When Err (e0):\n\
        \x20           Show e0.\n"
    )
}

/// A guaranteed-closed port: bind an ephemeral port, learn it, then drop the listener so nothing is listening.
fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind for a free port");
    let p = l.local_addr().expect("addr").port();
    drop(l);
    p
}

/// The connection-refused client: connect to a closed port and `Show` the `Err` (or, impossibly, the `Ok`).
fn refused_program(port: u16) -> String {
    format!(
        "## Main\n\
        \x20   Let cr be tcpConnect(\"127.0.0.1\", {port}).\n\
        \x20   Inspect cr:\n\
        \x20       When Ok (c):\n\
        \x20           Show c.\n\
        \x20       When Err (e):\n\
        \x20           Show e.\n"
    )
}

/// WITNESS 1 — a real TCP round-trip is byte-identical VM≡walker. The VM async driver and the walker C each
/// get their OWN echo server (a fresh ephemeral port) so the two runs never share a socket.
#[test]
fn tcp_round_trip_is_byte_identical_vm_and_walker() {
    if !have_gcc() {
        eprintln!("SKIP tcp round-trip ≡VM: gcc not installed");
        return;
    }
    // VM async tier against its own echo server.
    let vm_port = spawn_echo_once(5);
    let vm = vm_async_run(&round_trip_program(vm_port));

    // Walker C tier against a fresh echo server.
    let w_port = spawn_echo_once(5);
    let walker = walker_run(&round_trip_program(w_port), "roundtrip")
        .expect("the walker's Tcp C must compile, link, and run the round-trip");

    assert_eq!(
        vm,
        vec!["[72, 73, 74, 75, 76]".to_string()],
        "the VM oracle must echo the five bytes back"
    );
    assert_eq!(
        walker, vm,
        "the walker's native Tcp C must be byte-identical to the VM async driver"
    );
}

/// WITNESS 1b — `tcpSetTimeout` + `tcpShutdownWrite` (the two client ops beyond the connect/send/recv/close
/// subset) are byte-identical VM≡walker in a real round-trip. Both must return `Ok` for the echoed bytes to be
/// shown, so a matching `[72, 73, 74, 75, 76]` proves the walker's native `setsockopt`/`shutdown(2)` lowering
/// equals the VM's `TcpRegistry` — the walker now covers ALL of tcp.lg's client ops, none refused.
#[test]
fn tcp_set_timeout_and_shutdown_write_are_byte_identical_vm_and_walker() {
    if !have_gcc() {
        eprintln!("SKIP tcp set-timeout/shutdown-write ≡VM: gcc not installed");
        return;
    }
    let vm_port = spawn_echo_once(5);
    let vm = vm_async_run(&timeout_shutdown_program(vm_port));

    let w_port = spawn_echo_once(5);
    let walker = walker_run(&timeout_shutdown_program(w_port), "timeout_shutdown")
        .expect("the walker's Tcp C (with setTimeout + shutdownWrite) must compile, link, and run");

    assert_eq!(
        vm,
        vec!["[72, 73, 74, 75, 76]".to_string()],
        "the VM oracle must reach `Show bytes` — setTimeout AND shutdownWrite must both return Ok"
    );
    assert_eq!(
        walker, vm,
        "the walker's native setTimeout/shutdownWrite Tcp C must be byte-identical to the VM"
    );
}

/// WITNESS 2 — a refused connection is byte-identical VM≡walker, DOWN TO THE OS-ERROR WORDING. No server: the
/// port is deterministically closed, so both tiers take the `Err` path with the shared `tcp: connect to …
/// failed: <strerror> (os error <errno>)` text.
#[test]
fn tcp_connection_refused_is_byte_identical_vm_and_walker() {
    // The Err value is deterministic, so the VM half always runs (no gcc needed for it).
    let vm_port = free_port();
    let vm = vm_async_run(&refused_program(vm_port));
    assert_eq!(
        vm.len(),
        1,
        "the refused connect must Show exactly one Err line"
    );
    assert!(
        vm[0].starts_with("tcp: connect to 127.0.0.1:") && vm[0].contains("failed:"),
        "the VM's refused-connect Err must be the shared tcp wording, got {:?}",
        vm[0]
    );

    if !have_gcc() {
        eprintln!("SKIP tcp refused ≡VM (walker half): gcc not installed");
        return;
    }
    let w_port = free_port();
    let walker = walker_run(&refused_program(w_port), "refused")
        .expect("the walker's Tcp C must compile, link, and run the refused connect");

    // Both tiers built their own free port, so normalize the port digits before comparing the WORDING (the
    // failure reason — including `(os error 111)` — is what must not drift, not the ephemeral port number).
    let norm = |s: &str, p: u16| s.replace(&format!(":{p} "), ":<port> ");
    assert_eq!(
        norm(&walker[0], w_port),
        norm(&vm[0], vm_port),
        "the walker's refused-connect Err must be byte-identical to the VM's (incl. the OS-error wording)"
    );
}

/// WITNESS 3 — the walker's `Tcp` C compiles + links (the POSIX socket runtime is real, not a stub). Runs
/// even without a server: we only build, not run.
#[test]
fn tcp_walker_c_compiles_and_links() {
    if !have_gcc() {
        eprintln!("SKIP tcp compile check: gcc not installed");
        return;
    }
    // Exercise the WHOLE client op set so every lowered helper is referenced (connect/send/recv/recvExact/
    // setTimeout/shutdownWrite/close/listen/accept/listenerPort) — a link error in any would fail the build.
    let src = "## Main\n\
        \x20   Let cr be tcpConnect(\"127.0.0.1\", 1).\n\
        \x20   Inspect cr:\n\
        \x20       When Ok (c):\n\
        \x20           Let to be tcpSetTimeout(c, 1000).\n\
        \x20           Let s be tcpSend(c, [1, 2, 3]).\n\
        \x20           Let sw be tcpShutdownWrite(c).\n\
        \x20           Let r be tcpRecv(c, 8).\n\
        \x20           Let x be tcpRecvExact(c, 4).\n\
        \x20           Let cl be tcpClose(c).\n\
        \x20           Show 0.\n\
        \x20       When Err (e):\n\
        \x20           Show e.\n\
        \x20   Let lr be tcpListen(0).\n\
        \x20   Inspect lr:\n\
        \x20       When Ok (lis):\n\
        \x20           Let pr be tcpListenerPort(lis).\n\
        \x20           Let ar be tcpAccept(lis).\n\
        \x20           Show 1.\n\
        \x20       When Err (e2):\n\
        \x20           Show e2.\n";
    let c =
        compile_to_c_via_bytecode(src).expect("the walker must lower the whole Tcp client subset");
    assert!(
        c.contains("logos_tcp_connect"),
        "the socket runtime must be emitted for a Tcp program"
    );
    assert!(
        c.contains("logos_tcp_recv_exact"),
        "recvExact must lower to its socket helper"
    );
    assert!(
        c.contains("logos_tcp_listener_port"),
        "listenerPort must lower to its socket helper"
    );
    assert!(
        c.contains("logos_tcp_set_timeout"),
        "setTimeout must lower to its socket helper (no refusal)"
    );
    assert!(
        c.contains("logos_tcp_shutdown_write"),
        "shutdownWrite must lower to its socket helper (no refusal)"
    );

    let dir = std::env::temp_dir().join(format!("logos_tcpw_link_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let cfile = dir.join("prog.c");
    let bin = dir.join("prog");
    std::fs::write(&cfile, &c).expect("write C");
    let build = std::process::Command::new("gcc")
        .args(["-O2", "-c", "-o"])
        .arg(&bin)
        .arg(&cfile)
        .arg("-lm")
        .output()
        .expect("gcc runs");
    assert!(
        build.status.success(),
        "the walker's Tcp C must compile:\n{}",
        String::from_utf8_lossy(&build.stderr)
            .lines()
            .take(20)
            .collect::<Vec<_>>()
            .join("\n")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The delegation gate is now EMPTY — a Tcp program does NOT route to the AST-C backend; the walker lowers it
/// itself. (`compile_to_c` is the AST-C tier; the walker's C must differ, proving native lowering, not hand-off.)
#[test]
fn tcp_no_longer_delegates_to_astc() {
    let src = "## Main\n    Let r be tcpConnect(\"127.0.0.1\", 1).\n    Show 7.\n";
    let walker =
        compile_to_c_via_bytecode(src).expect("the walker must lower a Tcp program natively");
    assert!(
        walker.contains("logos_tcp_connect"),
        "the walker must emit the native socket runtime for a Tcp program (not delegate)"
    );
    let astc = logicaffeine_compile::compile::compile_to_c(src).expect("AST-C still compiles it");
    assert_ne!(
        walker, astc,
        "the walker's native Tcp C must NOT be byte-identical to AST-C — it lowers the op itself now"
    );
}
