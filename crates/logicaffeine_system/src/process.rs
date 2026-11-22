//! Running another program, bounded — the engine half of `process.lg`'s `runProcess`.
//!
//! A run is a program, its arguments, a working directory, environment entries laid over the
//! inherited environment, a deadline and an output cap. Every engine reaches this one
//! implementation: the tree-walker and the VM through the shared builtin seam
//! (`BuiltinId::ProcessRun`), the compiled Rust tier through [`run_parts`], so the bounds, the
//! refusals and their wording cannot drift between tiers.
//!
//! # The promises
//!
//! * **No shell.** The program is executed directly; each argument reaches it exactly as written.
//! * **Bounded.** `timeout_ms` and `max_output` are mandatory and range-checked before anything
//!   starts. The deadline stops the run; the cap keeps the first `max_output` bytes of each stream
//!   while the rest is drained and discarded, so a chatty child never blocks on a full pipe.
//! * **Nothing outlives the run.** On Unix the program leads a process group of its own. When the
//!   leader exits — on its own or killed at the deadline — the whole group is sent `SIGKILL`
//!   *before* the leader is reaped, so the group id cannot have been recycled for an unrelated
//!   process. A straggler that left the group (`setsid`) and still holds an output pipe open is
//!   waited for at most [`DRAIN_GRACE`] after the leader is gone; the call never hangs on it.
//! * **Empty input.** Standard input is `/dev/null`: a program that reads it sees end-of-input.
//! * **Sealed tests start nothing.** Inside a `## Test` the sealed world refuses unless the test
//!   granted `This test may use the real processes.` ([`crate::testworld::intercept_process`]).
//!
//! A run that happened is `Ok` whatever its outcome — a non-zero exit or a deadline is the program's
//! answer. `Err` means the run never happened.

use std::time::Duration;

/// The longest deadline a run may name: one day.
pub const MAX_TIMEOUT_MS: i64 = 86_400_000;

/// The largest output cap a run may name, per stream: 1 GiB.
pub const MAX_OUTPUT: i64 = 1 << 30;

/// How long the run keeps reading output after its leader is gone, for a straggler that escaped
/// the process group but still holds a pipe open.
pub const DRAIN_GRACE: Duration = Duration::from_secs(2);

/// What one finished run answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finished {
    /// The exit code, or 128 plus the signal that stopped the program.
    pub exit_code: i64,
    /// Standard output, UTF-8 with invalid bytes replaced by U+FFFD, at most the cap.
    pub stdout: String,
    /// Standard error, likewise.
    pub stderr: String,
    /// Whether the deadline stopped the run.
    pub timed_out: bool,
    /// Whether either stream was longer than the cap and was cut there.
    pub truncated: bool,
}

impl Finished {
    /// The five parts `runProcessParts` answers, in order: exit code, stdout, stderr, timed-out,
    /// truncated — each as Text.
    pub fn parts(&self) -> Vec<String> {
        vec![
            self.exit_code.to_string(),
            self.stdout.clone(),
            self.stderr.clone(),
            self.timed_out.to_string(),
            self.truncated.to_string(),
        ]
    }
}

/// The sealed world's refusal, in the words every engine reports.
pub fn sealed_refusal() -> String {
    crate::testworld::PROCESS_SEALED.to_string()
}

/// Check every input of a run, in the order the caller wrote them, without starting anything.
/// Answers the environment as `(name, value)` pairs.
pub fn validate(
    program: &str,
    args: &[String],
    env: &[String],
    timeout_ms: i64,
    max_output: i64,
) -> Result<Vec<(String, String)>, String> {
    if !(1..=MAX_TIMEOUT_MS).contains(&timeout_ms) {
        return Err(format!(
            "runProcess: timeoutMs must be between 1 and {MAX_TIMEOUT_MS}, got {timeout_ms}"
        ));
    }
    if !(1..=MAX_OUTPUT).contains(&max_output) {
        return Err(format!(
            "runProcess: maxOutput must be between 1 and {MAX_OUTPUT}, got {max_output}"
        ));
    }
    if program.is_empty() {
        return Err("runProcess: the program is empty".to_string());
    }
    if program.contains('\0') {
        return Err("runProcess: the program holds a NUL byte".to_string());
    }
    for (i, a) in args.iter().enumerate() {
        if a.contains('\0') {
            return Err(format!("runProcess: argument {} holds a NUL byte", i + 1));
        }
    }
    let mut pairs = Vec::with_capacity(env.len());
    for entry in env {
        let Some((name, value)) = entry.split_once('=') else {
            return Err(format!(
                "runProcess: an environment entry is NAME=value, got {entry:?}"
            ));
        };
        if name.is_empty() {
            return Err(format!(
                "runProcess: an environment entry needs a name before '=', got {entry:?}"
            ));
        }
        if entry.contains('\0') {
            return Err(format!(
                "runProcess: an environment entry holds a NUL byte, got {entry:?}"
            ));
        }
        pairs.push((name.to_string(), value.to_string()));
    }
    Ok(pairs)
}

/// Run `program` with `args`, bounded; see the module docs for every promise.
pub fn run(
    program: &str,
    args: &[String],
    cwd: &str,
    env: &[String],
    timeout_ms: i64,
    max_output: i64,
) -> Result<Finished, String> {
    if let Some(refusal) = crate::testworld::intercept_process() {
        return Err(refusal);
    }
    let pairs = validate(program, args, env, timeout_ms, max_output)?;
    if !cwd.is_empty() && !std::path::Path::new(cwd).is_dir() {
        return Err(format!(
            "runProcess: the working directory '{cwd}' is not a directory"
        ));
    }
    imp::run(
        program,
        args,
        cwd,
        &pairs,
        Duration::from_millis(timeout_ms as u64),
        max_output as usize,
    )
}

/// `process.lg`'s `runProcessParts` on the compiled Rust tier: [`run`], answering the five parts.
/// `args` and `env` arrive as the AOT tier's borrowed `Seq of Text` parameter form.
pub fn run_parts(
    program: String,
    args: &[String],
    cwd: String,
    env: &[String],
    timeout_ms: i64,
    max_output: i64,
) -> Result<logicaffeine_data::LogosSeq<String>, String> {
    let finished = run(&program, args, &cwd, env, timeout_ms, max_output)?;
    Ok(logicaffeine_data::LogosSeq::from(finished.parts()))
}

/// The bytes one stream has delivered, capped.
struct Capped {
    bytes: Vec<u8>,
    cap: usize,
    truncated: bool,
}

impl Capped {
    fn new(cap: usize) -> Self {
        Capped { bytes: Vec::new(), cap, truncated: false }
    }

    fn take(&mut self, chunk: &[u8]) {
        let room = self.cap.saturating_sub(self.bytes.len());
        if chunk.len() > room {
            self.truncated = true;
        }
        self.bytes.extend_from_slice(&chunk[..chunk.len().min(room)]);
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }
}

#[cfg(unix)]
mod imp {
    use super::{Capped, Finished, DRAIN_GRACE};
    use std::io::Read;
    use std::os::fd::AsRawFd;
    use std::os::unix::process::{CommandExt, ExitStatusExt};
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    /// How long one `poll` waits before the loop looks at the leader and the clock again.
    const TICK_MS: i32 = 10;

    pub(super) fn run(
        program: &str,
        args: &[String],
        cwd: &str,
        env: &[(String, String)],
        timeout: Duration,
        cap: usize,
    ) -> Result<Finished, String> {
        let mut command = Command::new(program);
        command
            .args(args)
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        if !cwd.is_empty() {
            command.current_dir(cwd);
        }
        let started = Instant::now();
        let mut child = command
            .spawn()
            .map_err(|e| format!("runProcess: cannot start '{program}': {e}"))?;
        let deadline = started + timeout;
        let outcome = supervise(&mut child, deadline, cap);
        // Whatever happened above, the group is stopped and the leader reaped before returning.
        let pid = child.id() as libc::pid_t;
        kill_group(pid);
        let status = child
            .wait()
            .map_err(|e| format!("runProcess: cannot reap '{program}': {e}"))?;
        let (out, err, timed_out) = outcome;
        let exit_code = match (status.code(), status.signal()) {
            (Some(code), _) => code as i64,
            (None, Some(signal)) => 128 + signal as i64,
            (None, None) => 128,
        };
        Ok(Finished {
            exit_code,
            stdout: out.text(),
            stderr: err.text(),
            timed_out,
            truncated: out.truncated || err.truncated,
        })
    }

    /// Drain both pipes until the leader is gone and the pipes are closed (or the drain grace runs
    /// out), killing the group at the deadline. Leaves the leader UNREAPED: the caller kills the
    /// group and reaps, in that order.
    fn supervise(child: &mut Child, deadline: Instant, cap: usize) -> (Capped, Capped, bool) {
        let pid = child.id() as libc::pid_t;
        let mut out = Capped::new(cap);
        let mut err = Capped::new(cap);
        let mut stdout = child.stdout.take();
        let mut stderr = child.stderr.take();
        let mut timed_out = false;
        let mut leader_gone_at: Option<Instant> = None;
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            if stdout.is_none() && stderr.is_none() && leader_gone_at.is_some() {
                break;
            }
            let now = Instant::now();
            if leader_gone_at.is_none() && now >= deadline && !timed_out {
                timed_out = true;
                kill_group(pid);
            }
            if let Some(gone) = leader_gone_at {
                if now.duration_since(gone) >= DRAIN_GRACE {
                    break;
                }
            }
            if leader_gone_at.is_none() && leader_exited(pid) {
                // Stop everything still in the group before the pipes are drained to the end:
                // nothing the run started outlives it, and nothing left can keep a pipe open.
                kill_group(pid);
                leader_gone_at = Some(Instant::now());
            }
            let mut fds = Vec::with_capacity(2);
            if let Some(s) = &stdout {
                fds.push(libc::pollfd { fd: s.as_raw_fd(), events: libc::POLLIN, revents: 0 });
            }
            if let Some(s) = &stderr {
                fds.push(libc::pollfd { fd: s.as_raw_fd(), events: libc::POLLIN, revents: 0 });
            }
            if fds.is_empty() {
                // Both pipes are closed while the leader lives on: wait for it in ticks.
                std::thread::sleep(Duration::from_millis(TICK_MS as u64));
                continue;
            }
            let ready = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, TICK_MS) };
            if ready <= 0 {
                continue; // a tick passed (or EINTR): look at the leader and the clock again
            }
            let mut i = 0;
            if stdout.is_some() {
                if fds[i].revents != 0 && !read_into(&mut stdout, &mut out, &mut buf) {
                    stdout = None;
                }
                i += 1;
            }
            if stderr.is_some() && fds[i].revents != 0 && !read_into(&mut stderr, &mut err, &mut buf) {
                stderr = None;
            }
        }
        (out, err, timed_out)
    }

    /// Read what one ready pipe holds into its capped buffer. Answers false at end-of-stream or on
    /// an error, when the pipe is done.
    fn read_into<R: Read>(pipe: &mut Option<R>, into: &mut Capped, buf: &mut [u8]) -> bool {
        let Some(p) = pipe.as_mut() else { return false };
        loop {
            match p.read(buf) {
                Ok(0) => return false,
                Ok(n) => {
                    into.take(&buf[..n]);
                    return true;
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return false,
            }
        }
    }

    /// Whether the leader has exited, WITHOUT reaping it (`WNOWAIT`), so its pid — and therefore the
    /// group id — stays reserved until the group has been killed.
    fn leader_exited(pid: libc::pid_t) -> bool {
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let rc = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        rc == 0 && unsafe { info.si_pid() } == pid
    }

    /// `SIGKILL` to every process in the run's group. A group that is already empty is not an error.
    fn kill_group(pid: libc::pid_t) {
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
mod imp {
    use super::{Capped, Finished};
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    pub(super) fn run(
        program: &str,
        args: &[String],
        cwd: &str,
        env: &[(String, String)],
        timeout: Duration,
        cap: usize,
    ) -> Result<Finished, String> {
        let mut command = Command::new(program);
        command
            .args(args)
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if !cwd.is_empty() {
            command.current_dir(cwd);
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("runProcess: cannot start '{program}': {e}"))?;
        let drain = |pipe: Option<Box<dyn Read + Send>>| {
            std::thread::spawn(move || {
                let mut capped = Capped::new(cap);
                if let Some(mut p) = pipe {
                    let mut buf = vec![0u8; 64 * 1024];
                    while let Ok(n) = p.read(&mut buf) {
                        if n == 0 {
                            break;
                        }
                        capped.take(&buf[..n]);
                    }
                }
                capped
            })
        };
        let out = drain(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
        let err = drain(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
        let deadline = Instant::now() + timeout;
        let mut timed_out = false;
        let status = loop {
            if let Some(status) = child
                .try_wait()
                .map_err(|e| format!("runProcess: cannot wait for '{program}': {e}"))?
            {
                break status;
            }
            if Instant::now() >= deadline {
                timed_out = true;
                let _ = child.kill();
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let out = out.join().unwrap_or_else(|_| Capped::new(cap));
        let err = err.join().unwrap_or_else(|_| Capped::new(cap));
        Ok(Finished {
            exit_code: status.code().map(|c| c as i64).unwrap_or(137),
            stdout: out.text(),
            stderr: err.text(),
            timed_out,
            truncated: out.truncated || err.truncated,
        })
    }
}
