//! Running a program on the RISC-V tier the way every RISC-V suite does: the emitted assembly linked with
//! the runtime shim by `riscv64-linux-gnu-gcc`, then run under `qemu-riscv64` inside a bounded address space
//! and a bounded time, so a program that grows without end or never finishes fails in seconds and names
//! itself instead of taking the machine or the run with it. Also the sizes a benchmark program runs at.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use logicaffeine_compile::codegen_riscv;

/// The address space a run gets, in megabytes; qemu-user takes about 256 MB of it for itself.
pub const RUN_CAP_MB: u64 = 512;

/// How long a run may take, in seconds, before it is stopped and fails with exit status 124.
pub const RUN_SECONDS: u64 = 60;

/// One run's outcome.
pub struct RvRun {
    pub stdout: String,
    pub stderr: String,
    pub code: Option<i32>,
    pub success: bool,
}

impl RvRun {
    /// The exit status and stderr, for a failure message.
    pub fn describe(&self) -> String {
        format!("exit {:?}, stderr:\n{}", self.code, self.stderr)
    }
}

/// A linked binary in a temp directory of its own, removed when dropped.
pub struct Linked {
    dir: PathBuf,
    pub bin: PathBuf,
}

impl Drop for Linked {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Assemble and link `asm` statically with the runtime it links ([`codegen_riscv::runtime`]): the shim, and the
/// exact extension's value runtime when the program reaches it. A link failure panics with the assembler's report
/// and the assembly, labelled.
pub fn link(asm: &str, label: &str) -> Linked {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("logos_riscv_{}_{n}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let asm_path = dir.join("main.s");
    let rt_path = dir.join("logos_rt.c");
    let bin = dir.join("main");
    let runtime = codegen_riscv::runtime(asm)
        .unwrap_or_else(|why| panic!("[{label}] the value runtime did not build: {why}"));
    std::fs::write(&asm_path, asm).expect("write .s");
    std::fs::write(&rt_path, &runtime.c).expect("write runtime shim");
    let c = std::process::Command::new("riscv64-linux-gnu-gcc")
        .args(["-O2", "-static", "-o"])
        .arg(&bin)
        .arg(&asm_path)
        .arg(&rt_path)
        .args(&runtime.link_args)
        .output()
        .expect("run riscv64-linux-gnu-gcc");
    assert!(
        c.status.success(),
        "[{label}] riscv64-linux-gnu-gcc failed to assemble/link:\n{}\n\nAsm:\n{asm}",
        String::from_utf8_lossy(&c.stderr)
    );
    Linked { dir, bin }
}

/// Run `linked` under `qemu-riscv64` with `args` and `env`, its address space capped at [`RUN_CAP_MB`]
/// soft and hard and its time at [`RUN_SECONDS`]. The cap also goes to the `qemu-riscv64` wrapper that
/// `scripts/run-qemu-corpus-tests.sh` puts first on the `PATH`, which sets the same limit rather than try
/// to raise it.
pub fn run(linked: &Linked, args: &[&str], env: &[(&str, String)]) -> RvRun {
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c")
        .arg("ulimit -v \"$1\" && t=\"$2\" && shift 2 && exec timeout \"$t\" qemu-riscv64 \"$@\"")
        .arg("sh")
        .arg((RUN_CAP_MB * 1024).to_string())
        .arg(RUN_SECONDS.to_string())
        .arg(&linked.bin)
        .args(args)
        .env("LOGICAFFEINE_QEMU_MEM_MB", RUN_CAP_MB.to_string());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let r = cmd
        .output()
        .expect("run qemu-riscv64 under a bounded address space and time");
    RvRun {
        stdout: String::from_utf8_lossy(&r.stdout).to_string(),
        stderr: String::from_utf8_lossy(&r.stderr).to_string(),
        code: r.status.code(),
        success: r.status.success(),
    }
}

/// The workload sizes a benchmark program declares in its `sizes.txt`: whitespace-separated positive
/// integers, at least one. Anything else panics, naming the file.
pub fn declared_sizes(program_dir: &Path) -> Vec<u64> {
    let path = program_dir.join("sizes.txt");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let sizes: Vec<u64> =
        text.split_whitespace()
            .map(|w| {
                w.parse::<u64>().ok().filter(|&n| n > 0).unwrap_or_else(|| {
                    panic!("{}: `{w}` is not a positive integer", path.display())
                })
            })
            .collect();
    assert!(!sizes.is_empty(), "{} declares no size", path.display());
    sizes
}

/// The sizes a differential runs a benchmark program at: the `candidates` no larger than the smallest
/// workload the program declares. A differential runs a program small, so both oracles stay fast; a
/// program whose own smallest workload is small (a factorial or exponential one) stops below it.
pub fn differential_sizes(program_dir: &Path, candidates: &[u64]) -> Vec<String> {
    let smallest = *declared_sizes(program_dir).iter().min().unwrap();
    candidates
        .iter()
        .filter(|&&c| c <= smallest)
        .map(u64::to_string)
        .collect()
}
