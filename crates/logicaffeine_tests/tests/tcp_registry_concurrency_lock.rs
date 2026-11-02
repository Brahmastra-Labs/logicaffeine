//! ★ LOCK — one connection's blocking read does not freeze every other connection. ★
//!
//! The native TCP natives keep their streams in a process-global `Mutex<Registry>`, and `send`,
//! `recv` and `recv_exact` took that lock and then performed the blocking syscall **while still
//! holding it**. Registration and close need the same lock, so a single stalled peer froze the
//! whole registry: a second connection could not be opened, could not exchange a byte, and could
//! not even be CLOSED until the first socket's read deadline expired.
//!
//! ★ THE FILE ALREADY KNEW. `accept` re-takes the lock on every poll with the comment *"Lock per
//! poll so a long accept never wedges other connections."* The hazard was seen for `accept` and
//! not carried to the other three — this repository's most-repeated shape, a rule installed where
//! the problem was observed while the identical flaw waits in a parallel spelling.
//!
//! ★ WHY IT MATTERS BEYOND CONCURRENCY: the cancellation path is `tcpSetTimeout` /
//! `tcpShutdownWrite` / `tcpClose`, and all three take that same lock. So the operation meant to
//! rescue a hung connection queued behind the hang it was trying to rescue.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// A listener that accepts and then says nothing, so a reader on it blocks.
fn silent_peer() -> (u16, std::thread::JoinHandle<()>) {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = l.local_addr().unwrap().port();
    let h = std::thread::spawn(move || {
        // Hold the accepted socket open, sending nothing, until the test drops us.
        if let Ok((s, _)) = l.accept() {
            std::thread::sleep(Duration::from_secs(3));
            drop(s);
        }
    });
    (port, h)
}

/// A listener that echoes one small frame, so a second connection can complete quickly.
fn echo_peer() -> (u16, std::thread::JoinHandle<()>) {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = l.local_addr().unwrap().port();
    let h = std::thread::spawn(move || {
        if let Ok((mut s, _)) = l.accept() {
            let mut b = [0u8; 4];
            let n = s.read(&mut b).unwrap_or(0);
            let _ = s.write_all(&b[..n]);
        }
    });
    (port, h)
}

#[test]
fn a_stalled_read_does_not_block_a_different_connection() {
    let (slow_port, slow_h) = silent_peer();
    let (fast_port, fast_h) = echo_peer();

    // Connection A: opened, then parked in a blocking read that will not be answered.
    let a = logicaffeine_system::tcp::connect("127.0.0.1".into(), slow_port as i64)
        .expect("connect to the silent peer");
    // A generous deadline: the point is that B must not wait for it.
    logicaffeine_system::tcp::set_timeout(a, 2500).expect("arm A's deadline");

    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let _ = logicaffeine_system::tcp::recv(a, 16); // blocks until A's deadline
        let _ = tx.send(());
    });

    // Give the reader time to be genuinely inside the syscall holding whatever it holds.
    std::thread::sleep(Duration::from_millis(300));

    // Connection B: a whole independent exchange, on a different socket.
    let started = Instant::now();
    let b = logicaffeine_system::tcp::connect("127.0.0.1".into(), fast_port as i64)
        .expect("B must be able to connect while A is parked");
    logicaffeine_system::tcp::send(b, &[104, 105]).expect("B must be able to send");
    let got = logicaffeine_system::tcp::recv_exact(b, 2).expect("B must be able to read its echo");
    logicaffeine_system::tcp::close(b).expect("B must be able to CLOSE — the cancellation path");
    let took = started.elapsed();

    assert_eq!(
        got.iter().collect::<Vec<_>>(),
        vec![104, 105],
        "B's echo came back"
    );
    assert!(
        took < Duration::from_millis(1500),
        "B's whole exchange took {took:?} — it waited for A's read deadline, which means the \
         registry lock is held across the blocking syscall. A stalled peer must not freeze every \
         other connection, and must not freeze the very calls used to cancel it."
    );

    let _ = rx.recv_timeout(Duration::from_secs(5));
    let _ = reader.join();
    let _ = logicaffeine_system::tcp::close(a);
    let _ = slow_h.join();
    let _ = fast_h.join();
}
