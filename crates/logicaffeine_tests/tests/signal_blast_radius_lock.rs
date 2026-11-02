//! ════════════════════════════════════════════════════════════════════════════
//! SIGNAL BLAST-RADIUS LOCK — nothing in this repo may signal what it does not own.
//!
//! ★ THE INCIDENT THIS LOCKS. `crates/logicaffeine_tests/tests/bind_address_lock.rs` tore
//!   its test server down with
//!
//!       Command::new("kill").arg("-9").arg(format!("-{}", child.id()))
//!
//!   which reads as "SIGKILL the child's process group" — and that IS what it means to
//!   a shell. It is not what it means to the BINARY. `Command::new` never uses a shell;
//!   it execs `/usr/bin/kill` (procps-ng), and procps parses a leading-`-` argument as
//!   an OPTION CLUSTER, keeping only the FIRST DIGIT and discarding the rest. The pid
//!   is never re-read. Confirmed by strace against procps-ng 4.0.4:
//!
//!       kill -0 -1746942   ->   kill(-1, 0)     ← every process this uid may signal
//!       kill -0 -2746942   ->   kill(-2, 0)
//!       kill -0 -87654     ->   kill(-8, 0)
//!
//!   `kill(-1, SIGKILL)` is POSIX for EVERY PROCESS THE CALLER MAY SIGNAL. So whenever
//!   the child's pid began with the digit `1` — about 26% of the pid space at the
//!   default `pid_max` of 4194304 — running this suite killed every shell, editor, SSH
//!   session and agent belonging to the developer, in the same instant.
//!
//! ★ WHY IT SURVIVED. It cannot be reproduced by hand. bash's `kill` BUILTIN parses the
//!   identical argv CORRECTLY (`kill(-1746942, 0)`), so pasting the command into a
//!   terminal proves the opposite of the truth. Only an exec of the binary reaches the
//!   broken parser. And it fired on roughly one run in four, which reads as a flaky
//!   machine or a bad network — not as a bug in a teardown path.
//!
//! ★ THE INVARIANT. Code that can make the syscall never execs the `kill` BINARY: Rust
//!   signals through `libc::kill`, Python through `os.kill`/`os.killpg`. Signalling a
//!   process GROUP is spelled `libc::kill(-pid, SIG)`: one syscall, no argv parser
//!   standing between the intent and the kernel, and identical on Linux and macOS —
//!   where BSD `kill(1)` rejects the `--` separator that would otherwise repair procps.
//!   The rule is on the exec, not on its operands, because a builder's argv can be
//!   spelled in more ways than a scanner can read (`.args(["-KILL", &format!("-{pid}")])`
//!   on one line went unseen by an operand scan). A shell script uses its builtin, and
//!   there a signal flag (`-9`, `-TERM`, `-s`) is fine; an OPERAND that starts with `-`
//!   and a digit is not, because it is exactly the spelling that broke.

use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// This file quotes the forbidden spellings in order to describe them, so it is the
/// one file the scan must not read.
const SELF: &str = "signal_blast_radius_lock.rs";

/// Not this repo's source: build output, vendored trees, tool scratch. Scanning them
/// finds other people's bugs, not ours.
const SKIP_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    "vendor",
    "coverage",
    "mutants.out",
    "mutants.out.old",
    "external",
];

/// The machine-local directories at the root — run logs (which archive whole trees),
/// toolchains, model weights. Output, not authored source; a clone has none of them.
const ROOT_SKIP_DIRS: &[&str] = &["logs", "tools", "models"];

/// Everything that can actually invoke a program.
const SCAN_EXT: &[&str] = &["rs", "sh", "bash", "py", "yml", "yaml", "just", "mk"];

fn collect(dir: &Path, top: bool, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if p.is_dir() {
            let skipped = SKIP_DIRS.contains(&name.as_str())
                || name.starts_with('.')
                || (top && ROOT_SKIP_DIRS.contains(&name.as_str()));
            if !skipped {
                collect(&p, false, out);
            }
        } else if name != SELF
            && p.extension()
                .is_some_and(|x| SCAN_EXT.contains(&x.to_string_lossy().as_ref()))
        {
            out.push(p);
        }
    }
}

/// Is this token a NEGATIVE PID rather than a signal/flag? `-9`/`-TERM`/`-f`/`--` are
/// not; `-1234`, `-$pid`, `-"$pid"`, `-{}` (a `format!` hole), `-%d` (a printf hole) are.
/// The test is the first character after the dash and any quote: a digit or an
/// interpolation is a pid, a letter is a flag.
fn looks_like_negative_pid(tok: &str) -> bool {
    let Some(rest) = tok.strip_prefix('-') else {
        return false;
    };
    let rest = rest.trim_start_matches(['"', '\'']);
    rest.chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit() || matches!(c, '$' | '{' | '%'))
}

/// Given the operand tokens that follow a `kill`/`pkill`, return the first one that is
/// a negative pid. The FIRST dash-token is the signal spec (`-9`, `-s`) and is skipped;
/// everything after it is an operand, and an operand may never start with `-` + digit.
fn first_offender(tokens: &[String]) -> Option<String> {
    let mut signal_seen = false;
    for t in tokens {
        if !t.starts_with('-') {
            continue;
        }
        if !signal_seen {
            signal_seen = true; // the signal spec itself
            continue;
        }
        if looks_like_negative_pid(t) {
            return Some(t.clone());
        }
    }
    None
}

/// Shell/Python/YAML: split the line into words and read forward from each `kill`.
fn scan_shellish(line: &str) -> Option<String> {
    let toks: Vec<String> = line
        .split_whitespace()
        .map(|s| s.trim_matches('"').to_string())
        .collect();
    for (i, t) in toks.iter().enumerate() {
        let base = t.rsplit('/').next().unwrap_or(t);
        if base == "kill" || base == "pkill" || base == "killall" {
            // Stop at a shell separator — the next command's args are not ours.
            let rest: Vec<String> = toks[i + 1..]
                .iter()
                .take_while(|w| !matches!(w.as_str(), ";" | "|" | "||" | "&&" | "&" | ")"))
                .cloned()
                .collect();
            if let Some(bad) = first_offender(&rest) {
                return Some(bad);
            }
        }
    }
    None
}

/// Where an exec names its program: Rust's `Command::new(`, and the argv list opening a
/// Python exec call. A list handed to anything else (`ctl(['kill', cid])` driving
/// `docker kill`) names a subcommand, not the binary.
const EXEC_HEADS: &[&str] = &[
    "Command::new(",
    "run([",
    "call([",
    "check_output([",
    "Popen([",
    "execv([",
    "execvp([",
];

/// Programs that run a later word of their argv as the real program (`sudo -n kill …`,
/// `timeout 5 kill …`): the program an exec runs is the first word past them, their flags,
/// their numbers and their `NAME=value` assignments.
const WRAPPERS: &[&str] = &[
    "sudo", "doas", "env", "nice", "ionice", "chrt", "timeout", "setsid", "nohup", "taskset",
    "stdbuf", "xargs", "command", "exec",
];

/// The words of an exec's argv as written on one line after its head: each string literal
/// as `Some`, and `None` for anything computed (`sys.executable`, `str(pid)`, `format!`),
/// past which nothing can be read statically. A builder's `.arg(` / `.args([` and all
/// punctuation only separate words.
fn argv_words(rest: &str) -> Vec<Option<String>> {
    let mut words = Vec::new();
    let mut chars = rest.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' || c == '\'' {
            words.push(Some(chars.by_ref().take_while(|d| *d != c).collect()));
        } else if c.is_ascii_alphanumeric() || c == '_' {
            let mut ident = String::from(c);
            while let Some(&d) = chars
                .peek()
                .filter(|d| d.is_ascii_alphanumeric() || matches!(d, '_' | ':' | '.'))
            {
                ident.push(d);
                chars.next();
            }
            if ident != "arg" && ident != "args" {
                words.push(None);
            }
        }
    }
    words
}

/// Rust/Python: the program of an exec that runs the `kill` binary, whatever path reaches it
/// (`kill`, `/bin/kill`, `/usr/bin/kill`) and whatever wrappers stand in front of it.
fn execs_kill_binary(line: &str) -> Option<String> {
    let heads = EXEC_HEADS
        .iter()
        .flat_map(|h| line.match_indices(h).map(|(i, m)| i + m.len()));
    for start in heads {
        let program = argv_words(&line[start..]).into_iter().find(|w| match w {
            Some(w) => {
                let wrapper = WRAPPERS.contains(&w.rsplit('/').next().unwrap_or(w));
                let numeric = !w.is_empty() && w.chars().all(|c| c.is_ascii_digit());
                !(wrapper || numeric || w.starts_with('-') || w.contains('='))
            }
            None => true,
        });
        if let Some(Some(program)) = program {
            if program.rsplit('/').next() == Some("kill") {
                return Some(program);
            }
        }
    }
    None
}

/// Every offence in one file, as `(line, what)`. The file's extension picks the rules.
fn offences_in(path: &str, src: &str) -> Vec<(usize, String)> {
    let code = path.ends_with(".rs") || path.ends_with(".py");
    let mut found = Vec::new();
    for (n, line) in src.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("//") || t.starts_with('#') || t.starts_with("*") {
            continue; // prose describing the bug is not the bug
        }
        if code {
            if let Some(program) = execs_kill_binary(line) {
                found.push((
                    n + 1,
                    format!(
                        "`{program}` exec'd — signal through the syscall, never the kill BINARY"
                    ),
                ));
            }
        }
        if let Some(bad) = scan_shellish(line) {
            found.push((n + 1, format!("`{bad}` passed to the kill BINARY")));
        }
    }
    found
}

/// ★ THE SCANNER SEES EVERY SPELLING THAT HAS FIRED. Each `BAD` line is one that reached
/// `kill(1)` with a negative pid (the first two verbatim from the tree), or the same exec
/// in another language; each `GOOD` line signals through the syscall or a builtin and
/// must stay legal. A scanner that goes blind to any of them fails here, not on a box
/// whose every session just died.
#[test]
fn the_scanner_flags_every_spelling_that_reaches_the_kill_binary() {
    const BAD: &[(&str, &str)] = &[
        ("tier_parity/mod.rs", "            let _ = Command::new(\"kill\").args([\"-KILL\", &format!(\"-{pid}\")]).stderr(Stdio::null()).status();"),
        ("bind_address_lock.rs", "        let _ = Command::new(\"kill\")\n            .arg(\"-9\")\n            .arg(format!(\"-{}\", self.0.id()))\n            .status();"),
        ("teardown.rs", "    let _ = Command::new(\"/usr/bin/kill\").arg(\"-TERM\").arg(pid.to_string()).status();"),
        ("reap.py", "    subprocess.run([\"kill\", \"-9\", f\"-{pgid}\"], check=False)"),
        ("reap.py", "    subprocess.call(['/bin/kill', '-KILL', str(pid)])"),
        ("scoreboard.py", "        subprocess.run([\"sudo\", \"-n\", \"kill\", \"-KILL\", f\"-{p.pid}\"], capture_output=True)"),
        ("teardown.rs", "    let _ = Command::new(\"sudo\").args([\"-n\", \"kill\", \"-KILL\", &format!(\"-{pgid}\")]).status();"),
        ("teardown.rs", "    let _ = Command::new(\"timeout\").arg(\"5\").arg(\"/usr/bin/kill\").arg(\"-TERM\").arg(pid.to_string()).status();"),
        ("reap.py", "    os.system(\"sudo -n kill -KILL -%d\" % pgid)"),
        ("reap.sh", "  kill -KILL -$pgid 2>/dev/null"),
        ("reap.sh", "  sudo -n kill -KILL -\"$pgid\""),
        ("reap.sh", "  /usr/bin/kill -9 -1234567"),
    ];
    const GOOD: &[(&str, &str)] = &[
        ("teardown.rs", "            unsafe { libc::kill(-pid, libc::SIGKILL) };"),
        ("teardown.rs", "        let _ = Command::new(\"pkill\").arg(\"-9\").arg(\"-f\").arg(\"--\").arg(dir).status();"),
        ("reap.py", "    os.killpg(pgid, signal.SIGKILL)"),
        ("reap.py", "    os.kill(pid, signal.SIGTERM)"),
        ("export.py", "   if current['State']['Running']:ctl(['kill','--signal=KILL',cid])"),
        ("export.py", "    subprocess.run([\"docker\", \"kill\", \"--signal=KILL\", cid], check=False)"),
        ("scoreboard.py", "        subprocess.run([\"sudo\", \"-n\", sys.executable, \"-c\", KILL_GROUP, str(p.pid)], capture_output=True)"),
        ("scoreboard.py", "    p = subprocess.Popen([\"sudo\", \"-n\", \"taskset\", \"-c\", \"2\", \"perf\", \"stat\", \"--\", checker])"),
        ("reap.sh", "  kill -TERM \"$pid\" 2>/dev/null"),
        ("reap.sh", "  kill -0 $pid 2>/dev/null && echo alive"),
    ];
    let missed: Vec<String> = BAD
        .iter()
        .filter(|(path, src)| offences_in(path, src).is_empty())
        .map(|(path, src)| format!("{path}: {}", src.trim()))
        .collect();
    let flagged: Vec<String> = GOOD
        .iter()
        .filter(|(path, src)| !offences_in(path, src).is_empty())
        .map(|(path, src)| format!("{path}: {}", src.trim()))
        .collect();
    assert!(
        missed.is_empty() && flagged.is_empty(),
        "the kill-binary scanner is blind to a spelling that SIGKILLs every process the uid owns:\n  {}\n\
         and flags a spelling that is safe:\n  {}",
        missed.join("\n  "),
        flagged.join("\n  ")
    );
}

/// ★ THE LOAD-BEARING ASSERTION. No file anywhere hands a negative pid to `kill(1)`.
#[test]
fn no_source_file_hands_a_negative_pid_to_the_kill_binary() {
    let root = repo_root();
    let mut files = Vec::new();
    collect(&root, true, &mut files);
    assert!(
        files.len() > 100,
        "the scan found only {} files — it is not reaching the tree",
        files.len()
    );

    let mut offences: Vec<String> = Vec::new();
    for f in &files {
        let Ok(src) = fs::read_to_string(f) else {
            continue;
        };
        let rel = f.strip_prefix(&root).unwrap_or(f).display().to_string();
        for (line, what) in offences_in(&rel, &src) {
            offences.push(format!("{rel}:{line}: {what}"));
        }
    }

    assert!(
        offences.is_empty(),
        "the `kill(1)` BINARY is exec'd, or handed a negative pid — procps keeps only the FIRST \
         DIGIT of a `-<pid>` operand, so `-1234567` becomes `kill(-1, …)`, which SIGKILLs every \
         process the uid owns (every shell, tmux server, SSH session and agent at once). Signal \
         through the syscall instead — `libc::kill(pid, SIG)` / `libc::kill(-pgid, SIG)` in Rust, \
         `os.kill`/`os.killpg` in Python — no argv parser, and correct on macOS too:\n  {}",
        offences.join("\n  ")
    );
}

/// ★ AND THE FIX ITSELF IS PINNED. A revert of `bind_address_lock.rs` to the shell-out
/// would otherwise pass the scan above only until someone re-typed the old spelling.
#[test]
fn the_server_teardown_still_signals_its_group_through_the_syscall() {
    let p = repo_root().join("crates/logicaffeine_tests/tests/bind_address_lock.rs");
    let src = fs::read_to_string(&p).expect("bind_address_lock.rs is readable");
    assert!(
        src.contains("libc::kill(-pid, libc::SIGKILL)"),
        "the `Server` teardown must signal its process group with `libc::kill`, not by \
         exec'ing `kill(1)` — see the header of this file for what that cost."
    );
}
