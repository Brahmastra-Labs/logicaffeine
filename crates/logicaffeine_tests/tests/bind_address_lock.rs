//! ★ LOCK — a LOGOS server can be reached from outside its own machine. ★
//!
//! Both tiers used to spell `format!("127.0.0.1:{port}")` at their own bind site.
//! `LOGOS_PORT` was honoured; the HOST was not, and no `LOGOS_HOST` existed. So a
//! `largo build --release` binary in a container, on Fly, Railway, Render or a k8s
//! pod **bound loopback and was unreachable from outside its own network
//! namespace** — while `docs/web.md` recommended exactly that deployment.
//!
//! ★ WHY ~600 WEB TESTS MISSED IT. Every one of them dials `127.0.0.1` from the
//! same host, where a loopback bind answers perfectly. The bug is invisible to any
//! test that does not cross a network boundary, and there was no such test. This
//! file is that test: it dials a **non-loopback address of this machine**, which a
//! loopback-bound server cannot answer no matter how correct everything else is.
//!
//! ★ AND IT LOCKS THE SEAM, NOT THE SYMPTOM. The address is composed in exactly one
//! place (`logicaffeine_system::http::server::env_bind_address`) and all three bind
//! sites — interpreter, `ui_bridge`, AOT codegen — call it. Two callers composing
//! "a host" with "a port" is two chances to compose it wrongly, and IPv6 is where
//! that bites: `::` joined to a port by a colon is `:::8080`, which is not an
//! address.

#[path = "cli_common/mod.rs"]
mod common;
use common::{largo, scaffold};

use std::io::{Read, Write};
use std::net::{IpAddr, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// A tiny server: one route, no pages, so this test is about the ADDRESS and
/// nothing else.
const SERVER_LG: &str = "# Bindsite\n\n\
    ## To handle (request: Request) -> Response:\n\
    \x20   Return plain(\"bound\").\n\n\
    ## Main\n\n\
    Serve on port 8080 with handle.\n";

/// The boot's budget in the server's OWN CPU seconds. `largo serve` interprets the program to come
/// up — ~26s of mostly single-threaded work here — so 120 CPU-seconds is a ~4.6x allowance for a
/// slower machine while remaining immune to how many other tests are competing for cores.
///
/// ★ THIS IS THE OPERATIVE LIMIT; wall time is now only a hang backstop. A wall-clock budget
/// cannot tell "wedged" from "starved", which is the only question worth asking, and no value fixes
/// that: 30s was raised to 180s for exactly this reason and still failed at 181-184s under suite
/// load. Failing because the server had plenty of CPU and never bound is a statement about the
/// PRODUCT. Failing because 180 seconds passed is a statement about the box.
const BOOT_CPU_BUDGET: f64 = 120.0;

/// The hang backstop, in wall time. Deliberately generous: it exists only so a child
/// that consumes no CPU AND never exits cannot wedge the run forever. The CPU budget above is what
/// actually decides pass or fail.
const BOOT_WALL_BACKSTOP: Duration = Duration::from_secs(900);

/// A non-loopback IPv4 address of THIS machine — the address a container's
/// neighbour, a load balancer or a phone on the LAN would dial.
///
/// Found by asking the routing table where it would send a packet, which needs no
/// name resolution and no interface enumeration: connecting a UDP socket sends
/// nothing, it just picks the source address the kernel would use.
fn outward_address() -> Option<IpAddr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("203.0.113.1:9").ok()?; // TEST-NET-3: reserved, never routed
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

/// A running server, killed however the test ends — **together with everything it
/// started**, which is the part that needed three attempts to get right.
///
/// ★ KILLING THE CHILD IS NOT ENOUGH, AND NEITHER IS ITS PROCESS GROUP. `largo serve`
/// builds the project and runs the RESULT, so the thing holding the port is a GRANDCHILD
/// in a process group of its own: `Child::kill` misses it, and so does a kill of this
/// child's group. Five of them survived one run of this file and sat holding ports with
/// their binaries already deleted. The project directory is unique per test and appears
/// in the grandchild's argv, so it is the one handle that certainly names it.
///
/// A test that leaks a server makes the next run flaky and the machine slowly worse.
struct Server(Child, PathBuf, std::sync::Arc<std::sync::Mutex<String>>);

impl Server {
    /// The port the server announced it bound (`Serving on http://…`), waiting while it is still
    /// booting by the same progress test `get_while` applies; `None` once the process has exited or
    /// spent its CPU budget without announcing. It is started on port 0, so the announcement is the
    /// only place its port is written.
    fn announced_port(&mut self) -> Option<u16> {
        let deadline = Instant::now() + BOOT_WALL_BACKSTOP;
        loop {
            if let Some(port) = common::http::announced_port(&self.2.lock().unwrap(), "") {
                return Some(port);
            }
            if !(self.still_booting() && self.cpu_seconds() < BOOT_CPU_BUDGET)
                || Instant::now() >= deadline
            {
                return None;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Has the spawned `largo` process exited? A boot wait that outlives the process is waiting for
    /// something that can no longer happen — see `get_while`.
    fn still_booting(&mut self) -> bool {
        matches!(self.0.try_wait(), Ok(None))
    }

    /// CPU seconds the spawned process GROUP has consumed — the `largo` child plus the server
    /// grandchild it execs, which is why this sums the group rather than reading one `/proc` entry
    /// (a running grandchild's time never appears in the parent's `cutime`, only a reaped one's).
    ///
    /// ★ THIS IS THE BOOT'S PROGRESS, AND WALL TIME NEVER WAS. `largo serve` INTERPRETS the program
    /// to come up: ~26s of mostly single-threaded work on an idle box. Under a suite saturating 32
    /// cores it gets a fraction of a core, so 180s of WALL time can contain barely any of that work —
    /// measured 2026-09-13, the test failed at 181.2s and 184.3s having never bound, while the same
    /// test needs 18-20s alone. A liveness check does not help either (tried, and it is why
    /// `still_booting` exists): the process is alive the whole time, just starved.
    ///
    /// So budget the thing that is actually the server's own doing. "It has had plenty of CPU and
    /// still has not bound" is a true statement about the PRODUCT; "180 seconds elapsed" is a
    /// statement about the machine.
    fn cpu_seconds(&self) -> f64 {
        let pgid = self.0.id() as i32;
        let mut ticks = 0f64;
        let Ok(dir) = std::fs::read_dir("/proc") else {
            return 0.0;
        };
        for e in dir.flatten() {
            let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<i32>().ok()) else {
                continue;
            };
            let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
                continue;
            };
            // `comm` is parenthesised and may contain spaces — split after the LAST ')'.
            let Some((_, rest)) = stat.rsplit_once(')') else {
                continue;
            };
            let f: Vec<&str> = rest.split_whitespace().collect();
            // After comm: [0]=state [1]=ppid [2]=pgrp … [11]=utime [12]=stime
            if f.len() > 12 && f[2].parse::<i32>().ok() == Some(pgid) {
                ticks += f[11].parse::<f64>().unwrap_or(0.0) + f[12].parse::<f64>().unwrap_or(0.0);
            }
        }
        ticks / 100.0 // CLK_TCK
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        // ★ THE GROUP KILL GOES THROUGH `libc::kill`, NEVER THROUGH `kill(1)`. ★
        //
        // This was once `Command::new("kill").arg("-9").arg(format!("-{}", self.0.id()))`,
        // and roughly one run in four it killed the developer's ENTIRE LOGIN SESSION —
        // every shell, editor and agent owned by the uid, all in the same instant.
        //
        // WHY. `Command::new` execs `/usr/bin/kill` (procps-ng), and procps parses a
        // leading-`-` argument as an OPTION CLUSTER, keeping only the FIRST DIGIT. The
        // remaining digits are discarded; the pid is never re-read. By strace, 4.0.4:
        //
        //     kill -0 -1746942   ->   kill(-1, 0)   ← "every process this uid may signal"
        //     kill -0 -2746942   ->   kill(-2, 0)
        //     kill -0 -87654     ->   kill(-8, 0)
        //
        // So any pid whose FIRST DIGIT IS `1` became `kill(-1, SIGKILL)`, which POSIX
        // defines as every process the caller may signal. With `pid_max = 4194304` about
        // 26% of pids begin with `1` — which is precisely why this read as a flaky
        // machine, or a bad network, rather than as a bug in this file.
        //
        // ★ AND IT CANNOT BE REPRODUCED BY HAND, which is how it survived. bash's `kill`
        // BUILTIN parses the very same argv CORRECTLY (`kill(-1746942, 0)`), so the
        // command pasted into a terminal does exactly the right thing. Only an exec of
        // the BINARY reaches the broken parser, and `Command::new` never uses a shell.
        //
        // A `--` separator repairs procps but is rejected by BSD `kill(1)` on macOS, so
        // the one spelling that is correct on both is the syscall itself. `libc` is
        // already a direct dependency of `logicaffeine_forge` and `logicaffeine_system`.
        let pid = self.0.id() as libc::pid_t;
        if pid > 1 {
            // SAFETY: `kill(2)` with a constant signal and a pid this process owns.
            // `process_group(0)` at spawn made the child its own group leader, so the
            // group id EQUALS the pid and `-pid` names that group and nothing else. The
            // guard is what keeps a hypothetical 0 or 1 from ever widening into "my own
            // group" or "everything" — the blast radius is bounded here, structurally,
            // rather than trusted to an argument parser. Failure is ESRCH or EPERM, and
            // a best-effort teardown ignores both.
            unsafe { libc::kill(-pid, libc::SIGKILL) };
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
        // `largo serve` builds the project and runs the RESULT, so the process actually
        // holding the port is a GRANDCHILD in a process group of its own, which the
        // group kill above does not reach. The per-test project dir is unique and
        // appears in that grandchild's argv, so it is the one handle that names it.
        // `--` so a path can never be read as options.
        let _ = Command::new("pkill")
            .arg("-9")
            .arg("-f")
            .arg("--")
            .arg(self.1.display().to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Dial `addr:port` and return the response, or `None` if nothing answered before
/// the deadline. A loopback-bound server answers `None` here for a LAN address.
fn get(addr: IpAddr, port: u16, timeout: Duration) -> Option<String> {
    get_while(addr, port, timeout, || true)
}

/// `get`, but the wait is PROGRESS-BASED: it keeps trying while `alive()` says the server is still
/// coming up, and gives up the moment it is not.
///
/// ★ WHY THIS EXISTS. A boot budget is not an assertion about the product — the assertions here are
/// about WHICH address gets bound. The budget only stops a slow machine turning a correct bind into a
/// red test, and its own comment says so. It was raised 30s → 180s for exactly that reason and the
/// test was put in the serialized `dev-server` group, and it STILL failed at 181.2s and 182.6s in the
/// 2026-09-13 baseline, because the suite drove the box to load 100-140 while `largo serve`
/// INTERPRETS the program to boot. The same test takes 15.1s on a quiet box.
///
/// Raising the number again would only move the next failure. A wall-clock budget cannot distinguish
/// "this server is wedged" from "this machine is busy", which is the only question worth asking. So
/// ask it directly: while the child is alive and has not yet bound, it is still booting — keep
/// waiting; once it has exited, no further waiting can help — fail NOW, which is strictly FASTER than
/// today for the case that actually indicates a bug. The deadline stays as a backstop against a child
/// that hangs forever without exiting.
fn get_while(
    addr: IpAddr,
    port: u16,
    timeout: Duration,
    alive: impl Fn() -> bool,
) -> Option<String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(mut s) =
            TcpStream::connect_timeout(&(addr, port).into(), Duration::from_millis(300))
        {
            let _ = s.write_all(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
            let mut out = String::new();
            let _ = s.read_to_string(&mut out);
            if !out.is_empty() {
                return Some(out);
            }
        }
        if !alive() {
            // The server process is gone: it will never bind, so waiting out the budget would only
            // delay a failure we can already prove.
            return None;
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Run the project with `largo run` on a port the system chooses (`LOGOS_PORT=0`), with the given host.
fn serve(dir: &std::path::Path, host: Option<&str>) -> Server {
    use std::os::unix::process::CommandExt;
    let mut cmd: Command = largo();
    // Its own process group, so `Server`'s drop can take the interpreted program with it.
    cmd.process_group(0);
    cmd.arg("run")
        .current_dir(dir)
        .env("LOGOS_PORT", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    match host {
        Some(h) => cmd.env("LOGOS_HOST", h),
        // Explicitly CLEARED, not merely unset in this process: an ambient
        // `LOGOS_HOST` in the developer's shell would otherwise make the
        // default-loopback assertion below pass or fail for the wrong reason.
        None => cmd.env_remove("LOGOS_HOST"),
    };
    let mut child = cmd.spawn().expect("largo serve");
    let printed = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let (mut pipe, sink) = (
        child.stdout.take().expect("stdout was piped"),
        std::sync::Arc::clone(&printed),
    );
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
    Server(child, dir.to_path_buf(), printed)
}

fn project(dir: &std::path::Path) {
    scaffold(dir, "bindsite");
    std::fs::write(dir.join("src/main.lg"), SERVER_LG).unwrap();
}

/// ★ THE LOAD-BEARING TEST — the one a deployment actually depends on.
#[test]
fn a_server_binds_an_address_a_container_can_be_reached_on() {
    let Some(outward) = outward_address() else {
        // A machine with no non-loopback interface cannot host this assertion.
        // Say so rather than passing silently — a green test that proved nothing
        // is how this bug survived in the first place.
        eprintln!("LOGOS_TOOL_SKIP: no non-loopback interface on this host");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let mut _s = serve(dir.path(), Some("0.0.0.0"));
    let port = _s.announced_port().unwrap_or_else(|| {
        panic!(
            "the server never announced the port it bound; it printed:\n{}",
            _s.2.lock().unwrap()
        )
    });

    let got = {
        let s = std::cell::RefCell::new(&mut _s);
        get_while(outward, port, BOOT_WALL_BACKSTOP, || {
            let mut g = s.borrow_mut();
            g.still_booting() && g.cpu_seconds() < BOOT_CPU_BUDGET
        })
    };
    assert!(
        got.as_deref().is_some_and(|r| r.contains("bound")),
        "with LOGOS_HOST=0.0.0.0 the server must answer on {outward}:{port} — the address a \
         container's neighbour, a load balancer or a phone on the LAN dials. A loopback-only \
         bind answers nothing here, which is exactly what a deployed binary used to do.\n\
         got: {got:?}"
    );
}

/// ★ AND THE DEFAULT MUST STAY LOOPBACK. Running a program is not consent to
/// publish it: a laptop on a café network would otherwise start serving to the
/// room. `LOGOS_HOST=0.0.0.0` is a decision somebody takes, never one they inherit.
#[test]
fn the_default_is_loopback_so_running_a_program_does_not_publish_it() {
    let Some(outward) = outward_address() else {
        eprintln!("LOGOS_TOOL_SKIP: no non-loopback interface on this host");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let mut _s = serve(dir.path(), None);
    let port = _s.announced_port().unwrap_or_else(|| {
        panic!(
            "the server never announced the port it bound; it printed:\n{}",
            _s.2.lock().unwrap()
        )
    });

    // It is up on loopback…
    let local = {
        let s = std::cell::RefCell::new(&mut _s);
        get_while(
            "127.0.0.1".parse().unwrap(),
            port,
            BOOT_WALL_BACKSTOP,
            || {
                let mut g = s.borrow_mut();
                g.still_booting() && g.cpu_seconds() < BOOT_CPU_BUDGET
            },
        )
    };
    assert!(
        local.as_deref().is_some_and(|r| r.contains("bound")),
        "the server must be up on loopback with no LOGOS_HOST set: {local:?}"
    );
    // …and NOT reachable from the outside. A short budget: the server is already
    // known to be running, so this is a refusal to answer, not a race with boot.
    let outside = get(outward, port, Duration::from_secs(2));
    assert!(
        outside.is_none(),
        "with no LOGOS_HOST the server must NOT answer on {outward}:{port} — the default binds \
         loopback so that running a program never publishes it to the network.\ngot: {outside:?}"
    );
}

/// ★ THE TIERS MUST NOT DRIFT. The deploy tier is the one a site actually ships
/// as, and it is the least-tested one — `compile_to_rust` is not exercised by the
/// 70 tests in `server_surface_lock`. So assert the generated Rust reaches the
/// SAME seam, and carries no address of its own.
#[test]
fn the_deploy_tier_composes_its_address_through_the_same_seam() {
    let rust = logicaffeine_compile::compile_to_rust(SERVER_LG).expect("the server compiles");
    assert!(
        rust.contains("env_bind_address"),
        "the AOT `Serve` lowering must bind through `env_bind_address`, or a deployed binary \
         honours `LOGOS_HOST` on one tier and not the other:\n{rust}"
    );
    assert!(
        !rust.contains("\"127.0.0.1:"),
        "the generated Rust must carry no hard-coded bind address — that literal is exactly the \
         bug this lock exists for:\n{rust}"
    );
    // A refusal hides in the returned string rather than in the `Err`, so a lock
    // that only checked `Ok` would be green on a program that cannot be built.
    assert!(
        !rust.contains("compile_error!"),
        "the server must actually lower, not answer Ok with a refusal inside it:\n{rust}"
    );
}
