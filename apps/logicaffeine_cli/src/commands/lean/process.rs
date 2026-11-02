//! Running a tool under a deadline, with bounded output.
//!
//! A deadline is a cancellation point: a tool still running when it passes is killed and waited
//! for, so no process outlives the run that started it.

use std::io::Read;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// What a finished tool left.
pub(super) struct Captured {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// stdout was longer than its cap; `stdout` holds the first `cap` bytes.
    pub stdout_truncated: bool,
}

pub(super) enum Failure {
    /// The tool could not be started.
    Spawn(std::io::Error),
    /// The deadline passed; the tool was killed.
    Deadline,
}

/// The longest stderr kept from any tool.
const STDERR_CAP: u64 = 16 << 20;

fn drain(
    mut source: impl Read + Send + 'static,
    cap: u64,
) -> std::thread::JoinHandle<(Vec<u8>, bool)> {
    std::thread::spawn(move || {
        let mut kept = Vec::new();
        let _ = (&mut source).take(cap).read_to_end(&mut kept);
        // Keep reading so the tool never blocks on a full pipe, but keep nothing more.
        let mut overflow = false;
        let mut sink = [0u8; 8192];
        while let Ok(n) = source.read(&mut sink) {
            if n == 0 {
                break;
            }
            overflow = true;
        }
        (kept, overflow)
    })
}

/// Run `command` to completion or until `deadline`, keeping at most `stdout_cap` bytes of stdout.
pub(super) fn run(
    mut command: Command,
    deadline: Instant,
    stdout_cap: u64,
) -> Result<Captured, Failure> {
    if Instant::now() >= deadline {
        return Err(Failure::Deadline);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(Failure::Spawn)?;
    let stdout = drain(child.stdout.take().expect("piped stdout"), stdout_cap);
    let stderr = drain(child.stderr.take().expect("piped stderr"), STDERR_CAP);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout.join();
                let _ = stderr.join();
                return Err(Failure::Deadline);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Failure::Spawn(e));
            }
        }
    };
    let (stdout, stdout_truncated) = stdout.join().unwrap_or_default();
    let (stderr, _) = stderr.join().unwrap_or_default();
    Ok(Captured {
        status,
        stdout,
        stderr,
        stdout_truncated,
    })
}
