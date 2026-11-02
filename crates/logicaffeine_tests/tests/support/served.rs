//! Dialling a server a test spawned: it binds a port the system chooses and says which, and the test dials
//! exactly that address; the server leads a process group of its own, and the whole group is killed on drop;
//! one-shot HTTP/1.1 requests frame the response by reading to EOF.
//!
//! A test never picks a port for a server to bind later. A port reserved and released is free for any other
//! process until the server binds it, and a site or a compiled program can take seconds to build first; on a
//! busy machine another test's server takes it, and a test that dials where it ASKED reads that server's pages.
//! `largo serve` moves to the next port when its own is taken, so the collision is silent. Every server here
//! is started on port `0` and dialed where its announcement says it listens.

#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A running server. It is spawned as the leader of a new process group, so everything it starts in turn —
/// the binary `largo run` builds, the backend `largo serve` proxies — is in the group too, and dropping the
/// handle kills the whole group and reaps the leader: a failed assertion never leaks a server, and no
/// process of it outlives the test holding a port, a pipe or an inherited lock. Its stdout and stderr are
/// drained as they are written, so a chatty server can never block on a full pipe.
pub struct Server {
    child: Child,
    stdout: Arc<Mutex<String>>,
    stderr: Arc<Mutex<String>>,
    port: u16,
}

impl Drop for Server {
    fn drop(&mut self) {
        // SAFETY: `kill` takes no pointers; the group id is the leader's PID, which stays reserved until the
        // `wait` below reaps it, so the signal cannot reach a group that reused the number.
        unsafe { libc::kill(-(self.child.id() as libc::pid_t), libc::SIGKILL) };
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Everything `pipe` writes, appended to `sink` as it arrives.
fn drain(mut pipe: impl Read + Send + 'static, sink: Arc<Mutex<String>>) {
    std::thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        while let Ok(n) = pipe.read(&mut chunk) {
            if n == 0 {
                break;
            }
            sink.lock()
                .unwrap()
                .push_str(&String::from_utf8_lossy(&chunk[..n]));
        }
    });
}

/// The port a server announced it bound. `largo serve` announces its public address on stderr
/// (`Serving <name> on http://<addr>`); a program's own `Serve` announces on stdout (`Serving on
/// http://<addr>`) — the full-stack dev server discards its backend child's stdout, so the two never
/// both name a public address.
pub fn announced_port(stdout: &str, stderr: &str) -> Option<u16> {
    let banner = |text: &str, largo: bool| {
        text.lines().find_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix("Serving ")?;
            let (before, url) = rest.split_once("http://")?;
            let names_a_server = if largo {
                before.ends_with(" on ") && before.len() > " on ".len()
            } else {
                before == "on "
            };
            if !names_a_server {
                return None;
            }
            let addr = url.split_whitespace().next()?;
            addr.rsplit_once(':')?
                .1
                .trim_end_matches('/')
                .parse::<u16>()
                .ok()
                .filter(|p| *p != 0)
        })
    };
    banner(stderr, true).or_else(|| banner(stdout, false))
}

impl Server {
    /// Spawn `cmd` (its stdin closed, its output drained) and return once it has announced the address it
    /// bound. A server that exits instead fails at once with what it printed; one that never announces fails
    /// after the wait, with what it printed.
    pub fn spawn(mut cmd: Command) -> Server {
        let mut child = cmd
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("the server spawns");
        let (stdout, stderr) = (
            Arc::new(Mutex::new(String::new())),
            Arc::new(Mutex::new(String::new())),
        );
        drain(
            child.stdout.take().expect("stdout was piped"),
            Arc::clone(&stdout),
        );
        drain(
            child.stderr.take().expect("stderr was piped"),
            Arc::clone(&stderr),
        );
        let deadline = Instant::now() + Duration::from_secs(300);
        let port = loop {
            let found = announced_port(&stdout.lock().unwrap(), &stderr.lock().unwrap());
            if let Some(port) = found {
                break port;
            }
            if matches!(child.try_wait(), Ok(Some(_))) {
                std::thread::sleep(Duration::from_millis(50));
                panic!(
                    "the server exited before announcing an address:\n--- stdout ---\n{}\n--- stderr ---\n{}",
                    stdout.lock().unwrap(),
                    stderr.lock().unwrap()
                );
            }
            assert!(
                Instant::now() < deadline,
                "the server announced no address in 300 s:\n--- stdout ---\n{}\n--- stderr ---\n{}",
                stdout.lock().unwrap(),
                stderr.lock().unwrap()
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        Server {
            child,
            stdout,
            stderr,
            port,
        }
    }

    /// The port this server announced.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The server's PID, which is also the id of the process group it leads.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// GET `path` from this server, waiting until it answers. A server that exits instead of answering fails
    /// at once with what it printed, rather than after the whole wait.
    pub fn get(&mut self, path: &str) -> Vec<u8> {
        let deadline = Instant::now() + Duration::from_secs(300);
        loop {
            if let Some(raw) = try_get(self.port, path) {
                return raw;
            }
            if self.exited() {
                panic!(
                    "the server exited instead of answering {path}:\n{}",
                    self.stderr()
                );
            }
            assert!(
                Instant::now() < deadline,
                "nothing answered {path} on port {}:\n{}",
                self.port,
                self.stderr()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Everything the server has written to stderr so far.
    pub fn stderr(&self) -> String {
        self.stderr.lock().unwrap().clone()
    }

    /// Everything the server has written to stdout so far.
    pub fn stdout(&self) -> String {
        self.stdout.lock().unwrap().clone()
    }

    /// Whether the server has already exited — a refusal to serve, rather than a server.
    pub fn exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }
}

/// One GET of `path`, or `None` when nothing is listening yet.
fn try_get(port: u16, path: &str) -> Option<Vec<u8>> {
    let mut s = TcpStream::connect(("127.0.0.1", port)).ok()?;
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes())
        .ok()?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).ok()?;
    (!raw.is_empty()).then_some(raw)
}
