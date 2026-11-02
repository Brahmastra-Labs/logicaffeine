//! no_self_matching_process_patterns_lock — no script may find or kill processes by a pattern
//! that can match the script itself.
//!
//! WHY THIS EXISTS. `pgrep -f PATTERN` and `pkill -f PATTERN` match the FULL COMMAND LINE of
//! every process. A shell running a script whose name, arguments, or heredoc body contains that
//! pattern therefore matches ITSELF. The two failure modes are:
//!
//!   LOUD    `pkill -f "openlane-run.sh systolic_pipe16"` killed the very shell that ran it,
//!           exit 144, mid-way through stopping a flow that was about to OOM.
//!   SILENT  `until ! pgrep -f "..."; do sleep; done` matched its own cmdline and span for
//!           25 minutes without ever running the job it was waiting for — and a wait loop that
//!           never fires is indistinguishable from a long-running job. A collision guard in
//!           scripts/signoff-processor-edges.sh had the same shape and once reported BUSY for
//!           over an hour on a completely idle box.
//!
//! The silent one is worse, and it is the reason this is a LOCK and not a comment. This repo
//! already documented the trap in three separate comments and two different agents still reached
//! for `-f` under time pressure, repeatedly, after reading the rule. Knowing it is not what
//! prevents it; the reach happens faster than the recall. So the mechanism has to refuse the
//! syntax rather than warn about it.
//!
//! THE SAFE ALTERNATIVES, all used in this repo:
//!   * `docker ps` — an OpenLane flow IS a container, and a container listing cannot match a shell.
//!   * `pgrep -x NAME` — matches the executable name exactly, never the arguments.
//!   * record the PID at launch and signal that PID.
//!   * `ps -eo args | grep '[f]low.tcl'` — the bracket keeps the grep from matching itself.

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn shell_scripts(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            shell_scripts(&p, out);
        } else if p.extension().and_then(|s| s.to_str()) == Some("sh") {
            out.push(p);
        }
    }
}

#[test]
fn no_script_greps_or_kills_by_full_command_line() {
    let base = root().join("scripts");
    let mut files = Vec::new();
    shell_scripts(&base, &mut files);
    files.sort();
    assert!(
        files.len() >= 20,
        "only {} shell scripts scanned — the walk is not working, so this lock would pass \
         vacuously",
        files.len()
    );

    let mut bad = Vec::new();
    for f in &files {
        let Ok(text) = std::fs::read_to_string(f) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            let t = line.trim_start();
            if t.starts_with('#') {
                continue; // documenting the trap is encouraged
            }
            let code = t.split('#').next().unwrap_or(t);
            if code.contains("pgrep -f")
                || code.contains("pkill -f")
                || code.contains("pgrep -af")
                || code.contains("pkill -9 -f")
            {
                bad.push(format!(
                    "  {}:{}  {}",
                    f.strip_prefix(root()).unwrap_or(f).display(),
                    i + 1,
                    code.trim()
                ));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} script line(s) match processes by FULL COMMAND LINE, which can match the script \
         itself:\n{}\n\nUse `docker ps` for container work, `pgrep -x NAME` for an exact \
         executable, a PID recorded at launch, or a bracketed `grep '[p]attern'`. A self-matching \
         wait loop reports BUSY forever on an idle box and looks exactly like a long-running job.",
        bad.len(),
        bad.join("\n")
    );
}
