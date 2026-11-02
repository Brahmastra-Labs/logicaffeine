//! ★ LOCK — a test dials a server where the server says it listens, never where the test asked.
//!
//! A port reserved with `bind("127.0.0.1:0")` and released is free for any other process until the server
//! binds it. A site builds and a program compiles first, for seconds on a busy machine, and a test suite is
//! a busy machine full of servers. When another test's server takes the port, `largo serve` moves to the
//! next one and says so, and a test that dials the port it asked for reads the other server's pages. That is
//! how `examples_lock::every_example_serves` once read another project's empty shell as marketing_site's
//! front page.
//!
//! So every server is started on port 0 and every tier ANNOUNCES the address it bound, with the one
//! function `logicaffeine_system::http::server::announce_serving` (`Serving on http://<addr>`), and the
//! suites dial that address through `support/served.rs`.
//!
//! | test | property |
//! |---|---|
//! | `every_tier_announces_the_port_it_bound` | the tree-walker (`largo run -i`), the VM (`run_vm_io_async`, in a re-executed process of its own) and the compiled tier (`largo run`) on `LOGOS_PORT=0` announce a real port and answer on it |
//! | `largo_serve_says_where_it_moved_when_its_port_is_taken` | asked for a port another server holds, a site moves on and announces where; the announcement reaches the site, the asked port reaches the other server |
//! | `the_compiled_tier_announces_the_port_it_bound_on_either_engine` | a `largo build` binary serves on the engine it is asked for — the io_uring engine is built into it on Linux — and on each announces a real port and answers on it |
//! | `a_dropped_server_takes_its_own_children_with_it` | a spawned server leads a process group, and dropping it leaves no process of the group alive |
//! | `no_suite_reserves_a_port_for_a_server_to_bind_later` | the reserve-and-release idiom appears only in the suites listed below, each for its stated reason; a listed suite that stops using it fails too |

#[path = "cli_common/mod.rs"]
mod common;
use common::http::{announced_port, body, Server};

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const ANSWERS: &str = "# Main\n\n\
    ## To handle (request: Request) -> Response:\n    \
        Return plain(\"answered \" + request's path).\n\n\
    ## Main\n\n\
    Serve on port 8080 with handle.\n";

fn backend(dir: &Path) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Largo.toml"),
        "[package]\nname = \"announced\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("src/main.lg"), ANSWERS).unwrap();
}

/// The variable that makes this test binary the VM tier's server process, naming the program it serves.
const VM_SERVER: &str = "LOGOS_SERVED_VM_SERVER";

/// The VM tier serves only in-process, through the library's `run_vm_io_async`, so its row re-executes this
/// test binary as that process: handed a program by `LOGOS_SERVED_VM_SERVER`, this test serves it on the VM
/// until it is killed. Run any other way, it returns at once.
#[test]
fn the_vm_tier_server_process() {
    let Ok(program) = std::env::var(VM_SERVER) else {
        return;
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let outcome = runtime.block_on(logicaffeine_compile::run_vm_io_async(&program));
    panic!("the VM tier stopped serving: {:?}", outcome.error);
}

/// The VM tier serving `program`: this binary, re-executed quietly (the terse format prints nothing before
/// the test's own output, so the announcement starts its line) and without capture.
fn vm_tier(program: &str) -> Command {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args([
        "--exact",
        "the_vm_tier_server_process",
        "--nocapture",
        "--quiet",
    ])
    .env(VM_SERVER, program);
    cmd
}

#[test]
fn every_tier_announces_the_port_it_bound() {
    let dir = tempfile::tempdir().unwrap();
    backend(dir.path());
    let largo = |flags: &[&str]| {
        let mut cmd = common::largo();
        cmd.args(flags).current_dir(dir.path());
        cmd
    };
    let tiers = [
        ("the tree-walker", largo(&["run", "-i"])),
        ("the VM", vm_tier(ANSWERS)),
        ("the compiled tier", largo(&["run"])),
    ];
    for (tier, mut cmd) in tiers {
        cmd.env("LOGOS_PORT", "0");
        let mut server = Server::spawn(cmd);
        assert!(
            server
                .stdout()
                .contains(&format!("Serving on http://127.0.0.1:{}", server.port())),
            "{tier} announces the address it bound on stdout:\n{}",
            server.stdout()
        );
        let raw = server.get("/here");
        assert_eq!(
            body(&raw),
            "answered /here",
            "{tier} answers at the port it announced"
        );
    }
}

/// A server on an OS-chosen port that answers every connection with `decoy`, until the test ends.
fn decoy() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut conn in listener.incoming().flatten() {
            let mut request = [0u8; 1024];
            let _ = conn.set_read_timeout(Some(Duration::from_secs(2)));
            let _ = conn.read(&mut request);
            let _ = conn.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\ndecoy",
            );
        }
    });
    port
}

fn get(port: u16, path: &str) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("something listens");
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes())
        .unwrap();
    let mut raw = Vec::new();
    let _ = s.read_to_end(&mut raw);
    body(&raw)
}

#[test]
fn largo_serve_says_where_it_moved_when_its_port_is_taken() {
    let taken = decoy();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("pages")).unwrap();
    std::fs::write(
        dir.path().join("Largo.toml"),
        "[package]\nname = \"own_site\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("pages/app.lg"),
        "## Page /\n\nView:\n    <h1>the site's own page</h1>\n",
    )
    .unwrap();
    let mut cmd = common::largo();
    cmd.args(["serve", "--port", &taken.to_string()])
        .current_dir(dir.path());
    let mut server = Server::spawn(cmd);
    assert_ne!(
        server.port(),
        taken,
        "the site cannot bind a port another server holds:\n{}",
        server.stderr()
    );
    let own = body(&server.get("/"));
    assert!(
        own.contains("the site's own page"),
        "the announced address reaches the site:\n{own}"
    );
    assert_eq!(
        get(taken, "/"),
        "decoy",
        "the port the test asked for answers whoever holds it"
    );
}

/// The compiled tier's server, built once per engine test run into the shared cache.
fn compiled_backend() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    backend(dir.path());
    let cache = std::env::temp_dir().join("logos_e2e_cache");
    let built = common::largo()
        .args(["build"])
        .current_dir(dir.path())
        .env("CARGO_TARGET_DIR", &cache)
        .output()
        .expect("largo build spawns");
    assert!(
        built.status.success(),
        "largo build:\n{}",
        common::stderr(&built)
    );
    let bin = dir.path().join("target/debug/announced");
    assert!(
        bin.exists(),
        "the build leaves its binary at {}",
        bin.display()
    );
    (dir, bin)
}

/// The engine a compiled server says it serves on — its `logos-http: serve: engine=` line on stderr, which
/// it writes before it binds, read once the drain has caught up with it.
fn engine_line(server: &Server) -> String {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(line) = server
            .stderr()
            .lines()
            .find(|l| l.starts_with("logos-http: serve: engine="))
        {
            return line.to_string();
        }
        assert!(
            Instant::now() < deadline,
            "the server named no engine:\n{}",
            server.stderr()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[ignore = "compiles an AOT binary via cargo (slow); run in the full suite"]
fn the_compiled_tier_announces_the_port_it_bound_on_either_engine() {
    let (_dir, bin) = compiled_backend();
    for engine in ["uring", "tokio"] {
        let mut cmd = Command::new(&bin);
        cmd.env("LOGOS_PORT", "0").env("LOGOS_HTTP_ENGINE", engine);
        let mut server = Server::spawn(cmd);
        let named = engine_line(&server);
        let served = named
            .strip_prefix("logos-http: serve: engine=")
            .and_then(|rest| rest.split_once(" ("));
        assert!(
            matches!(served, Some((kind, reason)) if kind == engine && reason.starts_with(&format!("LOGOS_HTTP_ENGINE={engine}"))),
            "a `largo build` binary serves on the engine it is asked for: {named}"
        );
        assert!(
            server
                .stdout()
                .contains(&format!("Serving on http://127.0.0.1:{}", server.port())),
            "the {engine} engine announces the address it bound:\n{}",
            server.stdout()
        );
        assert_eq!(
            body(&server.get("/here")),
            "answered /here",
            "the {engine} engine answers where it announced"
        );
    }
}

/// Whether any process of the group `group` is still alive.
fn group_alive(group: u32) -> bool {
    // SAFETY: signal 0 delivers nothing; `kill` only reports whether the group has a member.
    unsafe { libc::kill(-(group as libc::pid_t), 0) == 0 }
}

#[test]
fn a_dropped_server_takes_its_own_children_with_it() {
    let mut cmd = Command::new("sh");
    cmd.args([
        "-c",
        "sleep 600 & echo \"child $!\"; echo 'Serving on http://127.0.0.1:1'; wait",
    ]);
    let server = Server::spawn(cmd);
    let child: libc::pid_t = server
        .stdout()
        .lines()
        .find_map(|line| line.strip_prefix("child ")?.trim().parse().ok())
        .expect("the server names the child it started");
    // SAFETY: signal 0 delivers nothing.
    assert_eq!(
        unsafe { libc::kill(child, 0) },
        0,
        "the server's child is running while the server is"
    );
    let group = server.pid();
    drop(server);
    let deadline = Instant::now() + Duration::from_secs(10);
    // SAFETY: signal 0 delivers nothing.
    while group_alive(group) || unsafe { libc::kill(child, 0) } == 0 {
        assert!(
            Instant::now() < deadline,
            "a process the dropped server started outlived it for 10 s"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The suites that still reserve a port and release it, each with the reason the window cannot be closed
/// the same way. Every other suite dials an announced address.
const RESERVES_A_PORT: &[(&str, &str)] = &[
    ("backend_relay_lock.rs", "in-process servers (the engine entries and `interpret_for_ui_sync`) bind the port in this process microseconds after it is reserved"),
    ("http_interpreter.rs", "an in-process interpreted server binds the port in this process microseconds after it is reserved"),
    ("http_serve_bench_lock.rs", "an in-process interpreted server binds the port in this process microseconds after it is reserved"),
    ("http_serve_cross_tier.rs", "in-process interpreted and VM servers bind the port in this process microseconds after it is reserved"),
    ("http_workers.rs", "an in-process interpreted server binds the port in this process microseconds after it is reserved"),
    ("tcp_walker_lock.rs", "it needs a port NOTHING listens on, for a refused connect; std cannot hold a TCP port without listening on it"),
];

/// Whether `source` reserves a port and releases it: a `free_port` helper, or a listener bound to port 0
/// whose port is taken in the same expression and the listener dropped.
fn reserves_a_port(source: &str) -> bool {
    source.contains("fn free_port(")
        || source
            .lines()
            .any(|line| line.contains("bind(\"127.0.0.1:0\")") && line.contains(".port()"))
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|x| x == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn no_suite_reserves_a_port_for_a_server_to_bind_later() {
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let this = tests.join("served_address_lock.rs");
    let mut files = Vec::new();
    sources(&tests, &mut files);
    files.sort();
    let reserving: Vec<String> = files
        .iter()
        .filter(|f| **f != this)
        .filter(|f| reserves_a_port(&std::fs::read_to_string(f).unwrap_or_default()))
        .map(|f| f.strip_prefix(&tests).unwrap().display().to_string())
        .collect();
    let listed: Vec<&str> = RESERVES_A_PORT.iter().map(|(file, _)| *file).collect();
    let unlisted: Vec<&String> = reserving
        .iter()
        .filter(|f| !listed.contains(&f.as_str()))
        .collect();
    assert!(
        unlisted.is_empty(),
        "these suites reserve a port for a server to bind later; start the server on port 0 and dial the address \
         it announces (support/served.rs): {unlisted:?}"
    );
    let stale: Vec<&&str> = listed
        .iter()
        .filter(|f| !reserving.iter().any(|r| r == **f))
        .collect();
    assert!(
        stale.is_empty(),
        "these listed suites no longer reserve a port; delete their rows: {stale:?}"
    );
}

#[test]
fn the_announcement_is_read_from_either_stream() {
    assert_eq!(
        announced_port(
            "",
            "     Serving site on http://127.0.0.1:4242  (pages + static)"
        ),
        Some(4242)
    );
    assert_eq!(
        announced_port("Serving on http://0.0.0.0:5151\n", ""),
        Some(5151)
    );
    assert_eq!(
        announced_port(
            "Serving on http://127.0.0.1:7000\n",
            "     Serving site on http://127.0.0.1:7001  (x)"
        ),
        Some(7001),
        "largo's own banner names the public address"
    );
    assert_eq!(
        announced_port("Serving on http://127.0.0.1:0\n", ""),
        None,
        "port 0 is a request, not an address"
    );
    assert_eq!(
        announced_port("say Serving on http://127.0.0.1:9 later\n", ""),
        None,
        "only a banner line counts"
    );
}
