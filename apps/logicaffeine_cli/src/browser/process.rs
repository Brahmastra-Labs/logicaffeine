//! Starting Chrome with its DevTools pipe on fds 3 and 4, in a profile directory of its own.
//!
//! Every pipe end is created close-on-exec, so a process another thread spawns concurrently never
//! inherits one (an inherited write end would keep the pipe open after this process is gone, and
//! Chrome shuts down only when it reads end-of-file). In the child, between fork and exec, the two
//! ends Chrome needs are first duplicated ABOVE fd 10 and only then onto 3 and 4, so a source
//! descriptor that already sits at 3 or 4 is never overwritten before it is copied; `dup2` clears
//! close-on-exec on its target, which is what lets exactly those two survive into Chrome.

use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// The flags every harness browser runs with. Headless, no first-run UI, no background traffic,
/// and nothing that makes a page's behaviour depend on the machine it runs on.
///
/// `DeferRendererTasksAfterInput` is off: after an input event Chrome holds a page's deferrable
/// tasks — a WebSocket message among them — until the next frame or a timeout, and a page whose
/// clock the harness holds produces neither, so a relay message sent after a click would wait
/// forever.
const FLAGS: &[&str] = &[
    "--headless",
    "--remote-debugging-pipe",
    "--no-sandbox",
    "--disable-gpu",
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-background-networking",
    "--disable-background-timer-throttling",
    "--disable-backgrounding-occluded-windows",
    "--disable-renderer-backgrounding",
    "--disable-component-update",
    "--disable-default-apps",
    "--disable-extensions",
    "--disable-sync",
    "--disable-dev-shm-usage",
    "--disable-features=Translate,MediaRouter,OptimizationHints,AutofillServerCommunication,DeferRendererTasksAfterInput",
    "--password-store=basic",
    "--use-mock-keychain",
    "--mute-audio",
    "--hide-scrollbars",
    "--window-size=1280,900",
    "--force-color-profile=srgb",
];

pub(crate) struct Launched {
    pub(crate) child: Child,
    pub(crate) to_browser: File,
    pub(crate) from_browser: File,
    pub(crate) profile: PathBuf,
}

fn cloexec_pipe() -> std::io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0 as libc::c_int; 2];
    #[cfg(any(target_os = "linux", target_os = "android", target_os = "freebsd"))]
    let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "freebsd")))]
    let rc = unsafe {
        let rc = libc::pipe(fds.as_mut_ptr());
        if rc == 0 {
            libc::fcntl(fds[0], libc::F_SETFD, libc::FD_CLOEXEC);
            libc::fcntl(fds[1], libc::F_SETFD, libc::FD_CLOEXEC);
        }
        rc
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/// The directory a harness browser's profile lives in: `<tmp>/logos-browser-<pid>-<n>`.
fn profile_dir() -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    std::env::temp_dir().join(format!(
        "logos-browser-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Remove the profiles of harness processes that are no longer running.
///
/// A browser owned by a `static` is never dropped, so its process exits with the test binary and
/// Chrome follows when its pipe closes — leaving the profile behind. The next launch clears every
/// profile whose owning pid is gone.
fn reap_orphaned_profiles() {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(rest) = name.to_str().and_then(|n| n.strip_prefix("logos-browser-")) else {
            continue;
        };
        let Some(pid) = rest
            .split('-')
            .next()
            .and_then(|p| p.parse::<libc::pid_t>().ok())
        else {
            continue;
        };
        let gone = unsafe { libc::kill(pid, 0) } != 0
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
        if gone {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

pub(crate) fn launch(chrome: &Path) -> std::io::Result<Launched> {
    reap_orphaned_profiles();
    let profile = profile_dir();
    std::fs::create_dir_all(&profile)?;
    let log = File::create(profile.join("chrome.log"))?;

    // Chrome reads commands from fd 3 and writes to fd 4.
    let (chrome_reads, we_write) = cloexec_pipe()?;
    let (we_read, chrome_writes) = cloexec_pipe()?;
    let (r, w) = (chrome_reads.as_raw_fd(), chrome_writes.as_raw_fd());

    let mut cmd = Command::new(chrome);
    cmd.args(FLAGS)
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("about:blank")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log));
    // SAFETY: only async-signal-safe calls (`fcntl`, `dup2`) run between fork and exec.
    unsafe {
        cmd.pre_exec(move || {
            let high_r = libc::fcntl(r, libc::F_DUPFD_CLOEXEC, 10);
            let high_w = libc::fcntl(w, libc::F_DUPFD_CLOEXEC, 10);
            if high_r < 0 || high_w < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::dup2(high_r, 3) < 0 || libc::dup2(high_w, 4) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = cmd.spawn()?;
    drop(chrome_reads);
    drop(chrome_writes);
    Ok(Launched {
        child,
        to_browser: File::from(we_write),
        from_browser: File::from(we_read),
        profile,
    })
}

/// The last lines Chrome wrote to its log, for an error message.
pub(crate) fn log_tail(profile: &Path) -> String {
    let text = std::fs::read_to_string(profile.join("chrome.log")).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(12)..].join("\n")
}
