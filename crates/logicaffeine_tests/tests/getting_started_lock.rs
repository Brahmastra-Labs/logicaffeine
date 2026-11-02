//! The first fifteen minutes cannot break: `docs/getting-started.md` and the README's Quick start are
//! replayed exactly as a reader types them, with nothing but `largo` on the `PATH`.
//!
//! The replayer reads the page top to bottom and runs it as a script:
//!
//! - A ```` ```console ```` fence is a terminal session. `$ largo …`, `$ cd …` and `$ curl …` lines are
//!   commands; the lines under a command are output it must print, in that order, among whatever else
//!   it prints. A shown line ending in ` …` matches any printed line that begins with the rest.
//! - `$ largo serve` starts a server that lives until the page ends. The replayer gives it a free
//!   port and reads the page's `8080` as that port, in shown output and in `curl` URLs alike.
//! - An HTML comment `<!-- getting-started: <verb> <path> -->` directly above a fence ties the fence
//!   to a file in the reader's working tree: `generated` (the file `largo` wrote equals the fence),
//!   `write` (the reader types the fence into the file), or `excerpt` (the fence appears verbatim in
//!   the file).
//!
//! Every command must exit 0. The `PATH` holds one directory containing only `largo`, and the
//! environment is cleared down to a scratch `HOME`, so a step that quietly needs cargo, rustc, node or
//! the author's own config fails here instead of on a reader's first day.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test getting_started_lock`

#![cfg(all(unix, not(target_arch = "wasm32")))]

#[path = "cli_common/mod.rs"]
mod common;

use std::fs;
use std::io::Read;
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// The port a page names for `largo serve`, read as whatever free port the replay chose.
const PAGE_PORT: &str = "8080";

/// How a fence is tied to a file in the reader's tree.
#[derive(Debug, Clone, PartialEq)]
enum FileLink {
    Generated(String),
    Write(String),
    Excerpt(String),
}

/// One command of a console fence and the output shown under it.
#[derive(Debug, Clone, PartialEq)]
struct Step {
    line: usize,
    command: String,
    shown: Vec<String>,
}

/// One unit of the page's script, in page order.
#[derive(Debug, Clone, PartialEq)]
enum Action {
    Console(Vec<Step>),
    File {
        line: usize,
        link: FileLink,
        body: String,
    },
}

/// Parse a page into its script. A marker that is not directly above a fence, or that names an
/// unknown verb, is refused by name: a check that silently does not happen is the failure this lock
/// exists to prevent.
fn script(page: &str, name: &str) -> Vec<Action> {
    let lines: Vec<&str> = page.lines().collect();
    let mut actions = Vec::new();
    let mut pending: Option<(usize, FileLink)> = None;
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim();
        if let Some(rest) = t.strip_prefix("<!-- getting-started:") {
            let rest = rest
                .strip_suffix("-->")
                .unwrap_or_else(|| panic!("{name}:{} — unterminated marker", i + 1))
                .trim();
            let (verb, path) = rest
                .split_once(' ')
                .unwrap_or_else(|| panic!("{name}:{} — a marker names a verb and a path", i + 1));
            let path = path.trim().to_string();
            let link = match verb {
                "generated" => FileLink::Generated(path),
                "write" => FileLink::Write(path),
                "excerpt" => FileLink::Excerpt(path),
                other => panic!(
                    "{name}:{} — unknown marker verb `{other}` (generated, write, excerpt)",
                    i + 1
                ),
            };
            assert!(
                pending.is_none(),
                "{name}:{} — two markers above one fence",
                i + 1
            );
            pending = Some((i + 1, link));
            i += 1;
            continue;
        }
        if let Some(info) = t.strip_prefix("```") {
            let tag = info.split_whitespace().next().unwrap_or("").to_string();
            let start = i + 2;
            let mut body = String::new();
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                body.push_str(lines[i]);
                body.push('\n');
                i += 1;
            }
            if let Some((_, link)) = pending.take() {
                actions.push(Action::File {
                    line: start,
                    link,
                    body,
                });
            } else if tag == "console" {
                actions.push(Action::Console(console_steps(&body, name, start)));
            }
            i += 1;
            continue;
        }
        if !t.is_empty() {
            if let Some((at, _)) = &pending {
                panic!("{name}:{at} — a getting-started marker must sit directly above its fence");
            }
        }
        i += 1;
    }
    assert!(
        pending.is_none(),
        "{name} — a marker at the end of the page has no fence"
    );
    actions
}

fn console_steps(body: &str, name: &str, start: usize) -> Vec<Step> {
    let mut steps: Vec<Step> = Vec::new();
    for (k, line) in body.lines().enumerate() {
        if let Some(command) = line.strip_prefix("$ ") {
            steps.push(Step {
                line: start + k,
                command: command.trim().to_string(),
                shown: Vec::new(),
            });
        } else if line.trim().is_empty() {
            continue;
        } else {
            let step = steps
                .last_mut()
                .unwrap_or_else(|| panic!("{name}:{} — output before any `$ ` command", start + k));
            step.shown.push(line.to_string());
        }
    }
    steps
}

/// Split a command line into words, honouring double quotes.
fn words(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    for c in command.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            c => {
                cur.push(c);
                any = true;
            }
        }
    }
    assert!(!quoted, "unbalanced quote in `{command}`");
    if any {
        out.push(cur);
    }
    out
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether one shown line matches one printed line: equal up to whitespace runs, or, for a shown
/// line ending in ` …`, a prefix.
fn line_matches(shown: &str, printed: &str) -> bool {
    let shown = squash(shown);
    let printed = squash(printed);
    match shown.strip_suffix(" …") {
        Some(prefix) => printed.starts_with(prefix),
        None => printed == shown,
    }
}

/// The first shown line that does not appear, in order, among the printed lines.
fn first_missing<'a>(shown: &'a [String], printed: &str) -> Option<&'a str> {
    let mut lines = printed.lines();
    for want in shown {
        if !lines.any(|got| line_matches(want, got)) {
            return Some(want);
        }
    }
    None
}

/// The reader's machine: a working tree, a scratch `HOME`, and a `PATH` of one directory holding
/// only `largo`.
struct Reader {
    root: tempfile::TempDir,
    cwd: PathBuf,
    bin: PathBuf,
    home: PathBuf,
    server: Option<(Child, u16, PathBuf)>,
}

impl Drop for Reader {
    fn drop(&mut self) {
        if let Some((child, _, _)) = self.server.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Reader {
    fn new() -> Reader {
        let root = tempfile::tempdir().expect("a scratch tree");
        let bin = root.path().join(".path");
        let home = root.path().join(".home");
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(&home).unwrap();
        std::os::unix::fs::symlink(common_largo(), bin.join("largo"))
            .expect("link largo onto the PATH");
        let cwd = root.path().to_path_buf();
        Reader {
            root,
            cwd,
            bin,
            home,
            server: None,
        }
    }

    fn command(&self, args: &[String]) -> Command {
        let mut cmd = Command::new(self.bin.join("largo"));
        cmd.args(args)
            .current_dir(&self.cwd)
            .env_clear()
            .env("PATH", &self.bin)
            .env("HOME", &self.home)
            .env("NO_COLOR", "1")
            .stdin(Stdio::null());
        cmd
    }

    /// Run one step and hold what it printed against what the page shows under it.
    fn run(&mut self, step: &Step, name: &str) {
        let argv = words(&step.command);
        let at = format!("{name}:{} `{}`", step.line, step.command);
        match argv.first().map(String::as_str) {
            Some("cd") => {
                assert_eq!(argv.len(), 2, "{at} — `cd` takes one directory");
                assert!(step.shown.is_empty(), "{at} — `cd` prints nothing");
                let next = self.cwd.join(&argv[1]);
                assert!(next.is_dir(), "{at} — no directory {next:?}");
                self.cwd = next.canonicalize().unwrap();
                assert!(
                    self.cwd
                        .starts_with(self.root.path().canonicalize().unwrap()),
                    "{at} — left the scratch tree"
                );
            }
            Some("largo") if argv.get(1).map(String::as_str) == Some("serve") => {
                self.serve(&argv[1..], step, &at)
            }
            Some("largo") => {
                let printed = self.capture(&argv[1..], &at);
                if let Some(missing) = first_missing(&step.shown, &printed) {
                    panic!("{at} — the page shows\n    {missing}\nbut `largo` printed:\n{printed}");
                }
            }
            Some("curl") => {
                assert_eq!(argv.len(), 2, "{at} — the replayer reads `curl <url>`");
                let (port, path) = self.dial(&argv[1], &at);
                let raw = get(port, &path)
                    .unwrap_or_else(|| panic!("{at} — nothing answered on port {port}"));
                let body = common::http::body(&raw);
                assert_eq!(
                    common::http::status(&raw),
                    200,
                    "{at} — status {}:\n{}",
                    common::http::status(&raw),
                    String::from_utf8_lossy(&raw)
                );
                if let Some(missing) = first_missing(&step.shown, &body) {
                    panic!(
                        "{at} — the page shows\n    {missing}\nbut the server answered:\n{body}"
                    );
                }
            }
            other => {
                panic!("{at} — the replayer runs `largo`, `cd` and `curl`; the page runs {other:?}")
            }
        }
    }

    /// Run `largo` and return what it printed, stdout and stderr interleaved in the order written.
    fn capture(&self, args: &[String], at: &str) -> String {
        let log = self.root.path().join(".out");
        let out = fs::File::create(&log).unwrap();
        let err = out.try_clone().unwrap();
        let status = self
            .command(args)
            .stdout(Stdio::from(out))
            .stderr(Stdio::from(err))
            .status()
            .unwrap_or_else(|e| panic!("{at} — largo does not start: {e}"));
        let printed = common::strip_ansi(&fs::read_to_string(&log).unwrap());
        assert!(status.success(), "{at} — exited {status}:\n{printed}");
        printed
    }

    fn serve(&mut self, args: &[String], step: &Step, at: &str) {
        assert!(
            self.server.is_none(),
            "{at} — a second `largo serve` while the first still runs"
        );
        let log = self.root.path().join(".serve");
        let out = fs::File::create(&log).unwrap();
        let err = out.try_clone().unwrap();
        // On a port the system chooses, read back from what the server announces: a port picked here and
        // bound later could be taken by any other process in between (`common::http`).
        let mut argv = args.to_vec();
        argv.extend(["--port".to_string(), "0".to_string()]);
        let child = self
            .command(&argv)
            .env("LOGOS_PORT", "0")
            .stdout(Stdio::from(out))
            .stderr(Stdio::from(err))
            .spawn()
            .unwrap_or_else(|e| panic!("{at} — largo serve does not start: {e}"));
        self.server = Some((child, 0, log.clone()));
        let deadline = Instant::now() + Duration::from_secs(300);
        loop {
            let printed = common::strip_ansi(&fs::read_to_string(&log).unwrap_or_default());
            let port = common::http::announced_port(&printed, &printed);
            let shown: Option<Vec<String>> = port.map(|port| {
                step.shown
                    .iter()
                    .map(|l| l.replace(PAGE_PORT, &port.to_string()))
                    .collect()
            });
            if let (Some(port), Some(shown)) = (port, shown.as_ref()) {
                self.server.as_mut().unwrap().1 = port;
                if get(port, "/").is_some() && first_missing(shown, &printed).is_none() {
                    return;
                }
            }
            let (child, _, _) = self.server.as_mut().unwrap();
            if let Ok(Some(status)) = child.try_wait() {
                panic!("{at} — `largo serve` exited ({status}) instead of serving:\n{printed}");
            }
            if Instant::now() >= deadline {
                let missing = match &shown {
                    Some(shown) => first_missing(shown, &printed)
                        .unwrap_or("(an answer on /)")
                        .to_string(),
                    None => "(the address it serves on)".to_string(),
                };
                panic!("{at} — the page shows\n    {missing}\nbut after 300 s `largo serve` printed:\n{printed}");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// The port and path a page's `curl` URL names, with the page's port read as the served one.
    fn dial(&self, url: &str, at: &str) -> (u16, String) {
        let (_, port, _) = self
            .server
            .as_ref()
            .unwrap_or_else(|| panic!("{at} — `curl` before `largo serve`"));
        let rest = url
            .strip_prefix("http://localhost:")
            .or_else(|| url.strip_prefix("http://127.0.0.1:"))
            .unwrap_or_else(|| panic!("{at} — the replayer dials http://localhost:<port>/…"));
        let (page_port, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
        assert_eq!(
            page_port, PAGE_PORT,
            "{at} — the page serves on {PAGE_PORT}"
        );
        (
            *port,
            if path.is_empty() {
                "/".to_string()
            } else {
                path.to_string()
            },
        )
    }

    fn file(&self, line: usize, link: &FileLink, body: &str, name: &str) {
        match link {
            FileLink::Generated(path) => {
                let got = fs::read_to_string(self.root.path().join(path)).unwrap_or_else(|e| {
                    panic!("{name}:{line} — `largo` did not write {path}: {e}")
                });
                assert_eq!(
                    got, body,
                    "{name}:{line} — the page shows {path} as `largo` does not write it"
                );
            }
            FileLink::Write(path) => {
                let target = self.root.path().join(path);
                assert!(
                    target.exists(),
                    "{name}:{line} — the page replaces {path}, which does not exist yet"
                );
                fs::write(&target, body).unwrap();
            }
            FileLink::Excerpt(path) => {
                let got = fs::read_to_string(self.root.path().join(path)).unwrap_or_else(|e| {
                    panic!("{name}:{line} — `largo` did not write {path}: {e}")
                });
                assert!(got.contains(body), "{name}:{line} — the page quotes {path}, and the file does not contain the quote");
            }
        }
    }
}

fn common_largo() -> PathBuf {
    let path = common::largo().get_program().to_owned();
    PathBuf::from(path)
        .canonicalize()
        .expect("the largo under test exists")
}

fn get(port: u16, path: &str) -> Option<Vec<u8>> {
    use std::io::Write;
    let mut s = TcpStream::connect(("127.0.0.1", port)).ok()?;
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes())
        .ok()?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).ok()?;
    (!raw.is_empty()).then_some(raw)
}

/// Replay a page and return how many commands and file links it proved.
fn replay(relative: &str) -> (usize, usize) {
    let path = repo_root().join(relative);
    let page = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {relative}: {e}"));
    let mut reader = Reader::new();
    let (mut commands, mut files) = (0, 0);
    for action in script(&page, relative) {
        match action {
            Action::Console(steps) => {
                for step in &steps {
                    reader.run(step, relative);
                    commands += 1;
                }
            }
            Action::File { line, link, body } => {
                reader.file(line, &link, &body, relative);
                files += 1;
            }
        }
    }
    (commands, files)
}

#[test]
fn the_getting_started_guide_runs_with_only_largo_on_the_path() {
    let (commands, files) = replay("docs/getting-started.md");
    assert!(
        commands >= 10,
        "the guide should walk a reader through at least ten commands, it has {commands}"
    );
    assert!(
        files >= 3,
        "the guide should tie its programs to the reader's files, it ties {files}"
    );
}

#[test]
fn the_readme_quick_start_runs_with_only_largo_on_the_path() {
    let (commands, _) = replay("README.md");
    assert!(
        commands >= 3,
        "the README's Quick start should be a console session, it has {commands} commands"
    );
}

#[test]
fn a_shown_line_must_be_printed_in_order() {
    let printed = "Created LOGOS project 'hello'\n  cd hello\n  largo run\n";
    let shown = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(
        first_missing(&shown(&["Created LOGOS project 'hello'"]), printed),
        None
    );
    assert_eq!(
        first_missing(&shown(&["largo run", "cd hello"]), printed),
        Some("cd hello"),
        "order is part of the claim"
    );
    assert_eq!(
        first_missing(&shown(&["Created LOGOS project 'world'"]), printed),
        Some("Created LOGOS project 'world'")
    );
    assert_eq!(
        first_missing(&shown(&["Created LOGOS …"]), printed),
        None,
        "` …` is a prefix"
    );
    assert_eq!(
        first_missing(&shown(&["Created LOGOS"]), printed),
        Some("Created LOGOS"),
        "without ` …` the line is whole"
    );
}

#[test]
fn the_script_reader_refuses_what_it_cannot_check() {
    let page = "<!-- getting-started: generated a/b.lg -->\n```logos\nShow 1.\n```\n\n```console\n$ largo new a\nCreated\n$ cd a\n```\n";
    let actions = script(page, "x.md");
    assert_eq!(actions.len(), 2);
    assert_eq!(
        actions[0],
        Action::File {
            line: 3,
            link: FileLink::Generated("a/b.lg".into()),
            body: "Show 1.\n".into()
        }
    );
    let Action::Console(steps) = &actions[1] else {
        panic!("a console fence is a session")
    };
    assert_eq!(steps[0].shown, vec!["Created".to_string()]);

    for (bad, why) in [
        (
            "<!-- getting-started: typed a.lg -->\n```logos\n```\n",
            "unknown marker verb",
        ),
        (
            "<!-- getting-started: write a.lg -->\nprose\n```logos\n```\n",
            "directly above",
        ),
        ("```console\nstray output\n```\n", "output before any"),
        ("<!-- getting-started: write a.lg -->\n", "has no fence"),
    ] {
        let err = std::panic::catch_unwind(|| script(bad, "x.md")).expect_err(why);
        let msg = err.downcast_ref::<String>().cloned().unwrap_or_default();
        assert!(
            msg.contains(why),
            "expected a refusal naming `{why}`, got {msg:?}"
        );
    }
    assert_eq!(
        words(r#"largo logic "Every man is mortal.""#),
        vec!["largo", "logic", "Every man is mortal."]
    );
}

#[test]
fn the_reader_has_no_toolchain() {
    let reader = Reader::new();
    let entries: Vec<String> = fs::read_dir(&reader.bin)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        entries,
        vec!["largo".to_string()],
        "the reader's PATH holds largo and nothing else"
    );
    let probe = reader
        .command(&["--version".to_string()])
        .output()
        .expect("largo runs from the reader's PATH");
    assert!(
        probe.status.success(),
        "{}",
        String::from_utf8_lossy(&probe.stderr)
    );
    for tool in ["cargo", "rustc", "node"] {
        let found = Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("command -v {tool}"))
            .env_clear()
            .env("PATH", &reader.bin)
            .output()
            .unwrap();
        assert!(
            !found.status.success(),
            "{tool} is reachable from the reader's PATH"
        );
    }
}
