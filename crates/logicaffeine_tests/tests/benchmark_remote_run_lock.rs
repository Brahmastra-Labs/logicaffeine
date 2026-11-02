//! ★ LOCK — `benchmarks/run-remote.sh` and its machine half `benchmarks/lib/remote.sh`, end to end on a stub host. ★
//!
//! A full run on another machine takes hours and is unattended, so the orchestration around it has to be right before
//! it starts. This lock drives the real scripts against a "remote" that is a directory on this machine: `ssh` runs the
//! command there, `rsync` maps `host:path` into it, and every system tool the machine half touches (`sudo`,
//! `apt-get`, `systemctl`, `snap`, the toolchain installers, the governor files, `/proc/meminfo`) is a stub or a
//! fixture that records what it was asked. `run.sh` is a stub that records the environment it was started in and
//! writes a results directory.
//!
//! Pinned down: `prepare` ships the tree without build products, and installs exactly the rustc, Node and clang the
//! published run recorded, beside the system's toolchains rather than over them; a run measures with those toolchains,
//! a memory cap of three quarters of the machine, every CPU on the performance governor, sleep masked and the package
//! updaters paused, and gives every one of them back when it ends, pass or fail; a second run is refused while one
//! is going; `fetch` refuses an unfinished run, files a finished one under `results/machines/<chip>/<id>/`, and judges
//! it with `compare-runs.py` against the published run and the chip's previous run, exiting as it does.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test benchmark_remote_run_lock`

#![cfg(target_os = "linux")]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tempfile::TempDir;

const REFERENCE_CPU: &str = "Intel(R) Core(TM) i9-14900K";
const BOX_CPU: &str = "Intel(R) Core(TM) i7-4790K CPU @ 4.00GHz";
const BOX_CHIP: &str = "intel-core-i7-4790k";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn write_exe(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn results(cpu: &str, logos_ms: f64) -> (Value, Value, Value) {
    let meta = json!({
        "date": "2026-11-02T00:00:00Z", "commit": "abc1234", "logos_version": "0.11.0", "cpu": cpu,
        "versions": {"rust": "rustc 1.96.0 (ac68faa20 2026-05-25)", "js": "v22.22.3",
                     "c_clang": "Ubuntu clang version 21.1.8 (++20251221)", "c": "gcc 13.3.0"},
        "environment": {"governor": "performance", "quiet_box": {"gate": "passed"}},
    });
    let t = |ms: f64| json!({"mean_ms": ms, "cv": 0.01});
    let main = json!({"schema_version": 1, "metadata": meta, "benchmarks": [
        {"id": "fib", "reference_size": "40", "collapse": null,
         "scaling": {"40": {"c": t(100.0), "logos_release": t(logos_ms)}}}]});
    let interp = json!({"schema_version": 1, "metadata": meta, "benchmarks": [
        {"id": "fib", "reference_size": "38", "scaling": {"38": {"js": t(200.0), "logos_interp": t(14.0)}}}]});
    let codec = json!({"schema_version": 2, "metadata": meta, "scenarios": [
        {"id": "ints", "kind": "fair", "rows": [
            {"codec": "postcard", "size": 3000, "enc_ns": 1000.0, "dec_ns": 1200.0},
            {"codec": "logos (varint)", "size": 3231, "enc_ns": 1992.0, "dec_ns": 1749.0}]}]});
    (main, interp, codec)
}

fn write_results(dir: &Path, r: &(Value, Value, Value)) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("latest.json"), r.0.to_string()).unwrap();
    std::fs::write(dir.join("latest-interp.json"), r.1.to_string()).unwrap();
    std::fs::write(dir.join("latest-codec.json"), r.2.to_string()).unwrap();
}

/// A checkout holding the scripts under test, a stub host, and the stubs every tool resolves to.
struct Rig {
    _tmp: TempDir,
    local: PathBuf,
    remote_home: PathBuf,
    stubs: PathBuf,
    sys: PathBuf,
    proc_root: PathBuf,
    calls: PathBuf,
}

const SSH_STUB: &str = r#"#!/usr/bin/env bash
while [ $# -gt 0 ]; do
    case "$1" in -o|-i|-p|-l) shift 2 ;; -*) shift ;; *) break ;; esac
done
echo "ssh $1" >> "$RIG_CALLS"
shift
cd "$RIG_REMOTE_HOME" && HOME="$RIG_REMOTE_HOME" exec bash -c "$*"
"#;

const RSYNC_STUB: &str = r#"#!/usr/bin/env bash
args=()
for a in "$@"; do
    case "$a" in
        -*) args+=("$a") ;;
        *:/*) args+=("${a#*:}") ;;
        *:*) args+=("$RIG_REMOTE_HOME/${a#*:}") ;;
        *) args+=("$a") ;;
    esac
done
echo "rsync $*" >> "$RIG_CALLS"
exec "$RIG_REAL_RSYNC" "${args[@]}"
"#;

const RECORDER: &str = r#"#!/usr/bin/env bash
echo "$(basename "$0") $*" >> "$RIG_CALLS"
case "$(basename "$0") $1 $2" in
    "systemctl is-active --quiet") [ "$3" = apt-daily.timer ] ;;
    *) exit 0 ;;
esac
"#;

/// curl: the three downloads remote.sh makes, served from fixtures.
const CURL_STUB: &str = r#"#!/usr/bin/env bash
out=""; url=""
while [ $# -gt 0 ]; do
    case "$1" in -o) out="$2"; shift 2 ;; -*) shift ;; *) url="$1"; shift ;; esac
done
echo "curl $url" >> "$RIG_CALLS"
emit() { if [ -n "$out" ]; then cat > "$out"; else cat; fi; }
case "$url" in
    https://sh.rustup.rs) emit < "$RIG_FIXTURES/rustup-init.sh" ;;
    https://nodejs.org/dist/*/SHASUMS256.txt) emit < "$RIG_FIXTURES/SHASUMS256.txt" ;;
    https://nodejs.org/dist/*/node-*-linux-x64.tar.xz) emit < "$RIG_FIXTURES/$(basename "$url")" ;;
    *) echo "curl stub: unexpected $url" >&2; exit 22 ;;
esac
"#;

/// The rustup-init the stub curl serves: installs a rustup that records its calls.
const RUSTUP_INIT: &str = r#"#!/bin/sh
mkdir -p "$CARGO_HOME/bin"
cat > "$CARGO_HOME/bin/rustup" <<'EOF'
#!/bin/sh
echo "rustup $*" >> "$RIG_CALLS"
EOF
chmod +x "$CARGO_HOME/bin/rustup"
echo "rustup-init $*" >> "$RIG_CALLS"
"#;

/// run.sh: records what a real run would measure under, then writes a results directory.
const RUN_STUB: &str = r#"#!/usr/bin/env bash
set -euo pipefail
gov="$(cat "$BENCH_SYS_ROOT/devices/system/cpu/cpu0/cpufreq/scaling_governor")"
{
    echo "PATH=$PATH"
    echo "RUSTUP_TOOLCHAIN=${RUSTUP_TOOLCHAIN:-}"
    echo "BENCH_MEMORY_MAX=${BENCH_MEMORY_MAX:-}"
    echo "BENCH_RESULTS_DIR=${BENCH_RESULTS_DIR:-}"
    echo "XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-}"
    echo "DBUS_SESSION_BUS_ADDRESS=${DBUS_SESSION_BUS_ADDRESS:-}"
    echo "governor=$gov"
} > "$RIG_RUN_ENV"
while [ -e "$RIG_HOLD" ]; do sleep 0.1; done
cp "$RIG_FIXTURES"/run/latest*.json "$BENCH_RESULTS_DIR/"
exit "${RIG_RUN_EXIT:-0}"
"#;

impl Rig {
    fn new() -> Rig {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let local = root.join("local");
        let remote_home = root.join("remote");
        let stubs = root.join("stubs");
        let fixtures = root.join("fixtures");
        let sys = root.join("sys");
        let proc_root = root.join("proc");
        let calls = root.join("calls.log");
        std::fs::create_dir_all(&remote_home).unwrap();
        std::fs::write(&calls, "").unwrap();

        let real = repo_root().join("benchmarks");
        let bench = local.join("benchmarks");
        for f in ["run-remote.sh", "compare-runs.py", "setup-local.sh", "lib/remote.sh", "lib/assemble.py", "lib/suite.py",
                  "lib/benchmarks.tsv", "lib/languages.tsv", "lib/compilers.tsv"] {
            let to = bench.join(f);
            std::fs::create_dir_all(to.parent().unwrap()).unwrap();
            std::fs::copy(real.join(f), &to).unwrap();
        }
        write_exe(&bench.join("run.sh"), RUN_STUB);
        write_results(&bench.join("results"), &results(REFERENCE_CPU, 50.0));
        for junk in ["target/debug/big.o", "benchmarks/bin/fib_c", "benchmarks/.logos-bench-target/release/largo",
                     "work/private.md", "apps/web/node_modules/pkg/index.js"] {
            let p = local.join(junk);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "build product").unwrap();
        }

        write_exe(&stubs.join("ssh"), SSH_STUB);
        write_exe(&stubs.join("rsync"), RSYNC_STUB);
        write_exe(&stubs.join("curl"), CURL_STUB);
        write_exe(&stubs.join("sudo"), "#!/usr/bin/env bash\nexec \"$@\"\n");
        for t in ["apt-get", "systemctl", "snap", "loginctl"] {
            write_exe(&stubs.join(t), RECORDER);
        }
        for t in ["gcc", "g++", "clang", "taskset", "go", "javac", "ruby", "zig", "nim", "hyperfine", "rustc", "bc", "wget",
                  "clang-21", "clang++-21", "llvm-ar-21", "llvm-nm-21", "llvm-ranlib-21", "ld.lld-21"] {
            write_exe(&stubs.join(t), "#!/bin/sh\necho \"$(basename \"$0\") fixture 1.0\"\n");
        }

        std::fs::create_dir_all(&fixtures).unwrap();
        write_exe(&fixtures.join("rustup-init.sh"), RUSTUP_INIT);
        let node_root = fixtures.join("node-v22.22.3-linux-x64");
        write_exe(&node_root.join("bin/node"), "#!/bin/sh\necho v22.22.3\n");
        let tarball = fixtures.join("node-v22.22.3-linux-x64.tar.xz");
        let st = Command::new("tar")
            .arg("-cJf").arg(&tarball).arg("-C").arg(&fixtures).arg("node-v22.22.3-linux-x64")
            .status().unwrap();
        assert!(st.success());
        let sum = Command::new("sha256sum").arg(&tarball).output().unwrap();
        let hex = String::from_utf8_lossy(&sum.stdout).split_whitespace().next().unwrap().to_string();
        std::fs::write(
            fixtures.join("SHASUMS256.txt"),
            format!("0000  node-v22.22.3-darwin-arm64.tar.gz\n{hex}  node-v22.22.3-linux-x64.tar.xz\n"),
        ).unwrap();
        write_results(&fixtures.join("run"), &results(BOX_CPU, 80.0));

        for cpu in 0..4 {
            let g = sys.join(format!("devices/system/cpu/cpu{cpu}/cpufreq/scaling_governor"));
            std::fs::create_dir_all(g.parent().unwrap()).unwrap();
            std::fs::write(g, "schedutil\n").unwrap();
        }
        std::fs::create_dir_all(&proc_root).unwrap();
        std::fs::write(proc_root.join("meminfo"), "MemTotal:       16777216 kB\nMemFree:  1 kB\n").unwrap();

        Rig { _tmp: tmp, local, remote_home, stubs, sys, proc_root, calls }
    }

    fn fixtures(&self) -> PathBuf {
        self.stubs.parent().unwrap().join("fixtures")
    }

    fn hold(&self) -> PathBuf {
        self.stubs.parent().unwrap().join("hold")
    }

    fn run_env(&self) -> PathBuf {
        self.stubs.parent().unwrap().join("run.env")
    }

    fn remote_root(&self) -> PathBuf {
        self.remote_home.join("logos-bench")
    }

    fn cmd(&self, args: &[&str], extra_env: &[(&str, &str)]) -> Output {
        let real_rsync = Command::new("sh").arg("-c").arg("command -v rsync").output().unwrap();
        let real_rsync = String::from_utf8_lossy(&real_rsync.stdout).trim().to_string();
        assert!(!real_rsync.is_empty(), "rsync must be installed to run this lock");
        let path = format!("{}:{}", self.stubs.display(), std::env::var("PATH").unwrap());
        let mut c = Command::new("bash");
        c.arg(self.local.join("benchmarks/run-remote.sh"))
            .args(args)
            .env("PATH", path)
            .env("RIG_CALLS", &self.calls)
            .env("RIG_REMOTE_HOME", &self.remote_home)
            .env("RIG_REAL_RSYNC", real_rsync)
            .env("RIG_FIXTURES", self.fixtures())
            .env("RIG_HOLD", self.hold())
            .env("RIG_RUN_ENV", self.run_env())
            .env("BENCH_SYS_ROOT", &self.sys)
            .env("BENCH_PROC_ROOT", &self.proc_root)
            .env_remove("BENCH_REMOTE_ROOT")
            .env_remove("XDG_RUNTIME_DIR")
            .env_remove("DBUS_SESSION_BUS_ADDRESS");
        for (k, v) in extra_env {
            c.env(k, v);
        }
        c.output().unwrap()
    }

    fn calls(&self) -> String {
        std::fs::read_to_string(&self.calls).unwrap()
    }

    fn governors(&self) -> Vec<String> {
        (0..4)
            .map(|cpu| {
                std::fs::read_to_string(self.sys.join(format!("devices/system/cpu/cpu{cpu}/cpufreq/scaling_governor")))
                    .unwrap()
                    .trim()
                    .to_string()
            })
            .collect()
    }

    fn prepare(&self) {
        let out = self.cmd(&["box", "prepare"], &[]);
        assert!(out.status.success(), "prepare failed:\n{}", explain(&out));
    }

    fn start(&self, env: &[(&str, &str)]) -> String {
        let out = self.cmd(&["box", "start"], env);
        assert!(out.status.success(), "start failed:\n{}", explain(&out));
        String::from_utf8_lossy(&out.stdout).trim().lines().last().unwrap().to_string()
    }

    fn wait_finished(&self, id: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let out = self.cmd(&["box", "status", id], &[]);
            let s = String::from_utf8_lossy(&out.stdout).to_string();
            let restored = self.remote_root().join("runs").join(id).join("restored").exists();
            if s.contains("finished ") && restored {
                return s;
            }
            assert!(Instant::now() < deadline, "run {id} never finished:\n{s}\n{}", explain(&out));
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

fn explain(out: &Output) -> String {
    format!(
        "exit {:?}\nstdout:\n{}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn prepare_ships_the_tree_without_build_products() {
    let rig = Rig::new();
    rig.prepare();
    let repo = rig.remote_root().join("repo");
    assert!(repo.join("benchmarks/lib/remote.sh").is_file());
    assert!(repo.join("benchmarks/results/latest.json").is_file());
    for junk in ["target", "benchmarks/bin", "benchmarks/.logos-bench-target", "work", "apps/web/node_modules"] {
        assert!(!repo.join(junk).exists(), "{junk} must not travel");
    }
}

#[test]
fn prepare_pins_the_published_run_s_toolchains_beside_the_system_s() {
    let rig = Rig::new();
    rig.prepare();
    let calls = rig.calls();
    assert!(calls.contains("rustup toolchain install 1.96.0"), "{calls}");
    assert!(calls.contains("rustup default 1.96.0"), "{calls}");
    assert!(calls.contains("curl https://nodejs.org/dist/v22.22.3/node-v22.22.3-linux-x64.tar.xz"), "{calls}");
    assert!(calls.contains("apt-get install -y -q llvm-21-linker-tools lld-21"), "{calls}");
    assert!(calls.contains("build-essential"), "{calls}");
    let tools = rig.remote_root().join("tools");
    let node = Command::new(tools.join("node/bin/node")).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&node.stdout).trim(), "v22.22.3");
    let clang = std::fs::read_link(tools.join("bin/clang")).unwrap();
    assert_eq!(clang, rig.stubs.join("clang-21"), "clang points at the pinned major's binary");
    assert!(!calls.contains("llvm.sh"), "clang-21 was already there: {calls}");
    let me = String::from_utf8(Command::new("id").arg("-un").output().unwrap().stdout).unwrap();
    assert!(
        calls.contains(&format!("loginctl enable-linger {}", me.trim())),
        "the login's user manager runs from boot, so run.sh's systemd-run --user scope has one: {calls}"
    );
}

#[test]
fn a_run_measures_with_the_pinned_toolchains_on_a_quiet_machine_and_gives_it_back() {
    let rig = Rig::new();
    rig.prepare();
    let id = rig.start(&[]);
    rig.wait_finished(&id);

    let env = std::fs::read_to_string(rig.run_env()).unwrap();
    let tools = rig.remote_root().join("tools");
    assert!(
        env.contains(&format!("PATH={}/bin:{}/node/bin:{}/cargo/bin:", tools.display(), tools.display(), tools.display())),
        "the pinned toolchains come first: {env}"
    );
    assert!(env.contains("RUSTUP_TOOLCHAIN=1.96.0"), "{env}");
    assert!(env.contains("BENCH_MEMORY_MAX=12G"), "three quarters of a 16 GiB machine: {env}");
    assert!(env.contains("governor=performance"), "{env}");
    let uid = String::from_utf8(Command::new("id").arg("-u").output().unwrap().stdout).unwrap();
    let runtime = format!("/run/user/{}", uid.trim());
    assert!(
        env.contains(&format!("XDG_RUNTIME_DIR={runtime}\n")),
        "an ssh login without PAM still reaches the user manager: {env}"
    );
    assert!(env.contains(&format!("DBUS_SESSION_BUS_ADDRESS=unix:path={runtime}/bus\n")), "{env}");
    assert!(env.contains(&format!("BENCH_RESULTS_DIR={}", rig.remote_root().join("runs").join(&id).display())), "{env}");

    assert_eq!(rig.governors(), vec!["schedutil"; 4], "every CPU's governor is given back");
    let calls = rig.calls();
    for want in [
        "systemctl mask --runtime sleep.target suspend.target hibernate.target hybrid-sleep.target",
        "systemctl stop apt-daily.timer",
        "snap refresh --hold",
        "systemctl unmask --runtime sleep.target suspend.target hibernate.target hybrid-sleep.target",
        "systemctl start apt-daily.timer",
        "snap refresh --unhold",
    ] {
        assert!(calls.contains(want), "missing `{want}`:\n{calls}");
    }
    assert!(!calls.contains("systemctl stop apt-daily-upgrade.timer"), "an inactive timer is left alone: {calls}");
    assert!(!calls.contains("systemctl start apt-daily-upgrade.timer"), "{calls}");
}

#[test]
fn a_failed_run_still_gives_the_machine_back() {
    let rig = Rig::new();
    rig.prepare();
    let id = rig.start(&[("RIG_RUN_EXIT", "3")]);
    let status = rig.wait_finished(&id);
    assert!(status.contains("exit=3"), "{status}");
    assert_eq!(rig.governors(), vec!["schedutil"; 4]);
    assert!(rig.calls().contains("snap refresh --unhold"));
}

#[test]
fn a_second_run_is_refused_while_one_is_going() {
    let rig = Rig::new();
    rig.prepare();
    std::fs::write(rig.hold(), "").unwrap();
    let first = rig.start(&[]);
    let deadline = Instant::now() + Duration::from_secs(30);
    while !rig.run_env().exists() {
        assert!(Instant::now() < deadline, "the first run never started");
        std::thread::sleep(Duration::from_millis(50));
    }
    let second = rig.cmd(&["box", "start"], &[]);
    assert_eq!(second.status.code(), Some(1), "{}", explain(&second));
    assert!(String::from_utf8_lossy(&second.stderr).contains(&first), "names the run in the way: {}", explain(&second));

    let early = rig.cmd(&["box", "fetch", &first], &[]);
    assert_eq!(early.status.code(), Some(1), "{}", explain(&early));
    assert!(String::from_utf8_lossy(&early.stderr).contains("has not finished"), "{}", explain(&early));

    std::fs::remove_file(rig.hold()).unwrap();
    rig.wait_finished(&first);
}

#[test]
fn fetch_files_the_run_by_chip_and_judges_it_against_the_published_run() {
    let rig = Rig::new();
    rig.prepare();
    let id = rig.start(&[]);
    rig.wait_finished(&id);
    let out = rig.cmd(&["box", "fetch", &id], &[]);
    assert_eq!(out.status.code(), Some(0), "{}", explain(&out));
    let dest = rig.local.join("benchmarks/results/machines").join(BOX_CHIP).join(&id);
    for f in ["latest.json", "latest-interp.json", "latest-codec.json", "status", "report.md", "verdict.json"] {
        assert!(dest.join(f).is_file(), "{f} filed under {}", dest.display());
    }
    let verdict: Value = serde_json::from_str(&std::fs::read_to_string(dest.join("verdict.json")).unwrap()).unwrap();
    assert_eq!(verdict["comparisons"].as_array().unwrap().len(), 1, "no previous run of this chip yet");
    assert_eq!(verdict["comparisons"][0]["kind"], "cross_chip");
    assert!(String::from_utf8_lossy(&out.stdout).contains("Verdict"), "{}", explain(&out));
}

#[test]
fn a_chip_s_second_run_is_also_judged_against_its_first() {
    let rig = Rig::new();
    rig.prepare();
    let first = rig.start(&[]);
    rig.wait_finished(&first);
    assert_eq!(rig.cmd(&["box", "fetch", &first], &[]).status.code(), Some(0));

    write_results(&rig.fixtures().join("run"), &results(BOX_CPU, 80.0 * 1.3));
    std::thread::sleep(Duration::from_millis(1100));
    let second = rig.start(&[]);
    assert_ne!(first, second);
    rig.wait_finished(&second);
    let out = rig.cmd(&["box", "fetch"], &[]);
    assert_eq!(out.status.code(), Some(1), "30% slower than its own first run regresses: {}", explain(&out));
    let dest = rig.local.join("benchmarks/results/machines").join(BOX_CHIP).join(&second);
    let verdict: Value = serde_json::from_str(&std::fs::read_to_string(dest.join("verdict.json")).unwrap()).unwrap();
    let comps = verdict["comparisons"].as_array().unwrap();
    assert_eq!(comps.len(), 2, "the published run and the chip's previous run");
    assert_eq!(comps[1]["kind"], "same_chip");
    assert!(comps[1]["baseline"]["path"].as_str().unwrap().ends_with(&first));
}
