//! One program, every backend, one observation.
//!
//! A LOGOS program has one meaning, and every backend the product ships must show exactly that meaning:
//! the same bytes on standard output, the same bytes on standard error, the same way of ending. This module
//! is the harness that asks. [`Observation`] is the whole observable surface of a run and is compared with
//! `==`, never trimmed or normalized, because a normalization is a place two backends are allowed to differ.
//! [`Backend`] lists every way the product can execute a program, and [`Backend::of_product`] and
//! [`Backend::of_emit_target`] tie that list to the product's own enumerations with total matches, so a
//! backend added to the product cannot be missing here.
//!
//! [`run`] drives one program on one backend and answers a [`Verdict`]. Only [`Verdict::Observed`] can agree
//! with the reference; a refusal, a missing toolchain and a backend this harness cannot drive are each
//! reported under their own name and none of them is a pass.
#![allow(dead_code)]

pub mod corpus;
pub mod docs;
pub mod exact;
pub mod kinds;
pub mod machine_sympathy;
pub mod rust_batch;
pub mod semantic;

#[path = "../support/process_group.rs"]
mod process_group;

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use logicaffeine_compile::compile;

/// The instant every backend reads for `today` and `now`: in-process through the thread's fixed clock, in a
/// compiled program through `LOGOS_FIXED_DAYS` and `LOGOS_FIXED_NANOS`.
pub const FIXED_DAYS: i64 = 20000;
pub const FIXED_NANOS: i64 = 20000 * 86400 * 1_000_000_000 + 45_296 * 1_000_000_000;

/// How long one backend may take to run one program before the run is recorded as a timeout.
pub const RUN_LIMIT: Duration = Duration::from_secs(120);

/// The address-space cap for a compiled program on the host (8 GiB), so a leak ends the run instead of the
/// machine.
pub const ADDRESS_SPACE_CAP: u64 = 8 << 30;

/// The address-space cap for a program under qemu: the bound every RISC-V differential runs under.
pub const QEMU_ADDRESS_SPACE_CAP: u64 = 512 << 20;

/// Everything a run can be seen to do.
#[derive(Clone, PartialEq, Eq)]
pub struct Observation {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub end: End,
}

/// How a run ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum End {
    /// Exit status 0.
    Finished,
    /// Exit status 1: the program stopped with the failure its standard error names.
    Failed,
    /// Any other ending: another exit status, a signal, a timeout. No LOGOS program ends this way.
    Abnormal(String),
}

impl Observation {
    /// An in-process run: every shown line followed by a newline on standard output, and a failure's
    /// reason followed by a newline on standard error.
    pub fn of_lines(lines: &[String], failure: Option<&str>) -> Self {
        let mut stdout = Vec::new();
        for line in lines {
            stdout.extend_from_slice(line.as_bytes());
            stdout.push(b'\n');
        }
        Self::of_stream(stdout, failure)
    }

    /// An in-process run whose engine already wrote the output stream (the bytecode VM's `output()`).
    pub fn of_stream(stdout: impl Into<Vec<u8>>, failure: Option<&str>) -> Self {
        match failure {
            None => Observation {
                stdout: stdout.into(),
                stderr: Vec::new(),
                end: End::Finished,
            },
            Some(why) => Observation {
                stdout: stdout.into(),
                stderr: format!("{why}\n").into_bytes(),
                end: End::Failed,
            },
        }
    }

    /// A process: its two streams exactly, and its exit status read as a LOGOS ending.
    pub fn of_process(status: ExitStatus, stdout: Vec<u8>, stderr: Vec<u8>) -> Self {
        use std::os::unix::process::ExitStatusExt;
        let end = match (status.code(), status.signal()) {
            (Some(0), _) => End::Finished,
            (Some(1), _) => End::Failed,
            (Some(code), _) => End::Abnormal(format!("exit status {code}")),
            (None, Some(signal)) => End::Abnormal(format!("killed by signal {signal}")),
            (None, None) => End::Abnormal("ended with no status".to_string()),
        };
        Observation {
            stdout,
            stderr,
            end,
        }
    }

    /// The first place two observations differ, in words.
    pub fn difference(&self, want: &Observation) -> Option<String> {
        if self.end != want.end {
            let said = |o: &Observation| {
                let text = String::from_utf8_lossy(&o.stderr);
                let line: String = text
                    .lines()
                    .map(str::trim)
                    .find(|l| !l.is_empty())
                    .unwrap_or("")
                    .chars()
                    .take(160)
                    .collect();
                if line.is_empty() {
                    String::new()
                } else {
                    format!(" ({line:?})")
                }
            };
            return Some(format!(
                "ended {:?}{} where the reference ended {:?}{}",
                self.end,
                said(self),
                want.end,
                said(want)
            ));
        }
        if self.stdout != want.stdout {
            return Some(format!(
                "stdout {}",
                first_difference(&self.stdout, &want.stdout)
            ));
        }
        if self.stderr != want.stderr {
            return Some(format!(
                "stderr {}",
                first_difference(&self.stderr, &want.stderr)
            ));
        }
        None
    }
}

impl std::fmt::Debug for Observation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Observation")
            .field("stdout", &String::from_utf8_lossy(&self.stdout))
            .field("stderr", &String::from_utf8_lossy(&self.stderr))
            .field("end", &self.end)
            .finish()
    }
}

/// Where two byte strings first part, with a window of each around it.
pub fn first_difference(got: &[u8], want: &[u8]) -> String {
    let at = got
        .iter()
        .zip(want)
        .position(|(a, b)| a != b)
        .unwrap_or(got.len().min(want.len()));
    let window = |s: &[u8]| {
        let from = at.saturating_sub(24);
        let to = (at + 40).min(s.len());
        format!("{:?}", String::from_utf8_lossy(&s[from.min(s.len())..to]))
    };
    format!(
        "differs at byte {at} of {} (reference {} bytes): got …{}…, reference …{}…",
        got.len(),
        want.len(),
        window(got),
        window(want)
    )
}

/// What one backend did with one program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// It ran, and this is everything it did.
    Observed(Observation),
    /// It declined the program: the backend refused it by name, or its toolchain rejected the code the
    /// backend emitted. The first line of what was said.
    Refused(String),
    /// This machine lacks the backend's toolchain or device. A counted skip, never a pass.
    Unavailable(String),
    /// The backend exists but this harness has no way to run a program on it yet. A hole in the lock.
    Unrun(String),
}

impl Verdict {
    pub fn label(&self) -> &'static str {
        match self {
            Verdict::Observed(_) => "OBSERVED",
            Verdict::Refused(_) => "REFUSED",
            Verdict::Unavailable(_) => "UNAVAILABLE",
            Verdict::Unrun(_) => "UNRUN",
        }
    }
}

/// Every way the product executes a program.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Backend {
    /// The tree-walking interpreter.
    TreeWalker,
    /// The bytecode VM, unoptimized, driven by whichever VM driver the program needs.
    Vm,
    /// The bytecode VM after the run-path optimizer at its top tier.
    VmOptimized,
    /// The bytecode VM with the copy-and-patch JIT installed: x86-64 machine code for every hot region.
    VmForge,
    /// Futamura projection 1 with the LOGOS partial evaluator (`pe_source`) hosted on the tree-walker: the
    /// residual program, decompiled, run on the VM.
    Projection1,
    /// The same projection with the partial evaluator hosted on the VM.
    Projection1OnVm,
    /// Projection 1 through the Rust optimizer and decompiler (`projection1_source`).
    Projection1Optimizer,
    /// Futamura projection 2: `pe_source` specializing `pe_mini` to the program, the residual run by the
    /// core interpreter.
    Projection2,
    /// Futamura projection 3: `pe_source` specializing `pe_bti` to the program.
    Projection3,
    /// `largo run --interpret`: the product's own dispatcher, as a user runs it.
    Interpret,
    /// `largo run` / `largo build`: the generated Rust under cargo's dev profile.
    RustDebug,
    /// `largo build --release`: the generated Rust under the shipped release profile.
    RustRelease,
    /// `largo emit c`: the AST → C backend, built with gcc.
    C,
    /// The bytecode → C op-walker, built with gcc.
    CBytecode,
    /// `largo emit wasm`: the built-in WebAssembly backend, run by its emitted Node host.
    Wasm,
    /// `largo emit wasm-linked`: the WebAssembly backend linked against the real `BigInt` runtime.
    WasmLinked,
    /// `largo emit wasm-dom`: a WebAssembly page and its browser host.
    WasmDom,
    /// `largo emit riscv`: RV64GC assembly, linked with its C runtime and run under qemu.
    RiscV,
    /// `largo emit cuda`: the whole program on one GPU thread.
    Cuda,
    /// `largo emit tenstorrent`: a single-Tensix-core TT-Metalium program.
    Tenstorrent,
    /// The gate-level soft core, through its cycle model.
    SoftCore,
    /// The spatial compute fabric: the whole program lowered to the block-walk engine and evaluated as the
    /// RTL runs it.
    Fabric,
    /// `largo emit analog`: the analog spatial fabric.
    Analog,
    /// `largo emit plasticine`: the open reconfigurable dataflow lane.
    Plasticine,
    /// `largo emit rdu`: the SambaNova RDU lane.
    Rdu,
}

impl Backend {
    pub const ALL: [Backend; 25] = [
        Backend::TreeWalker,
        Backend::Vm,
        Backend::VmOptimized,
        Backend::VmForge,
        Backend::Projection1,
        Backend::Projection1OnVm,
        Backend::Projection1Optimizer,
        Backend::Projection2,
        Backend::Projection3,
        Backend::Interpret,
        Backend::RustDebug,
        Backend::RustRelease,
        Backend::C,
        Backend::CBytecode,
        Backend::Wasm,
        Backend::WasmLinked,
        Backend::WasmDom,
        Backend::RiscV,
        Backend::Cuda,
        Backend::Tenstorrent,
        Backend::SoftCore,
        Backend::Fabric,
        Backend::Analog,
        Backend::Plasticine,
        Backend::Rdu,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Backend::TreeWalker => "tree-walker",
            Backend::Vm => "vm",
            Backend::VmOptimized => "vm-optimized",
            Backend::VmForge => "vm-forge",
            Backend::Projection1 => "futamura-p1",
            Backend::Projection1OnVm => "futamura-p1-on-vm",
            Backend::Projection1Optimizer => "futamura-p1-optimizer",
            Backend::Projection2 => "futamura-p2",
            Backend::Projection3 => "futamura-p3",
            Backend::Interpret => "largo-interpret",
            Backend::RustDebug => "rust-debug",
            Backend::RustRelease => "rust-release",
            Backend::C => "c",
            Backend::CBytecode => "c-bytecode",
            Backend::Wasm => "wasm",
            Backend::WasmLinked => "wasm-linked",
            Backend::WasmDom => "wasm-dom",
            Backend::RiscV => "riscv",
            Backend::Cuda => "cuda",
            Backend::Tenstorrent => "tenstorrent",
            Backend::SoftCore => "soft-core",
            Backend::Fabric => "fabric",
            Backend::Analog => "analog",
            Backend::Plasticine => "plasticine",
            Backend::Rdu => "rdu",
        }
    }

    /// The product's own list of bytecode-lowering backends, each mapped to its column. Total: a backend
    /// added to the product does not compile here until it has one.
    pub fn of_product(b: compile::Backend) -> Backend {
        match b {
            compile::Backend::C => Backend::CBytecode,
            compile::Backend::CFromAst => Backend::C,
            compile::Backend::RiscV => Backend::RiscV,
            compile::Backend::WebAssembly => Backend::Wasm,
            compile::Backend::SoftCore => Backend::SoftCore,
        }
    }

    /// `largo emit`'s targets, by the name the CLI accepts, each mapped to its column. `None` is an emit
    /// target this harness does not know, which the census lock reports as a missing column.
    pub fn of_emit_target(name: &str) -> Option<Backend> {
        Some(match name {
            "rust" => Backend::RustRelease,
            "c" => Backend::C,
            "riscv" => Backend::RiscV,
            "wasm" => Backend::Wasm,
            "wasm-linked" => Backend::WasmLinked,
            "wasm-dom" => Backend::WasmDom,
            "tenstorrent" => Backend::Tenstorrent,
            "cuda" => Backend::Cuda,
            "analog" => Backend::Analog,
            "plasticine" => Backend::Plasticine,
            "rdu" => Backend::Rdu,
            _ => return None,
        })
    }
}

/// A program under test: a stable identifier, its source and its arguments, exactly as every backend receives
/// them.
#[derive(Clone, Debug)]
pub struct Program {
    pub id: String,
    pub source: String,
    /// The arguments after the program's name: `item 2 of args()` is the first of them.
    pub args: Vec<String>,
}

impl Program {
    pub fn new(id: impl Into<String>, source: impl Into<String>) -> Self {
        Program {
            id: id.into(),
            source: source.into(),
            args: Vec::new(),
        }
    }

    /// The full argument vector an in-process engine receives: the program's name, then its arguments.
    pub fn argv(&self) -> Vec<String> {
        std::iter::once("main".to_string())
            .chain(self.args.iter().cloned())
            .collect()
    }

    /// The refusal of a backend that has no way to hand a program its arguments, when the program has any.
    fn needs_argv(&self, backend: Backend) -> Option<Verdict> {
        (!self.args.is_empty()).then(|| {
            Verdict::Refused(format!(
                "{} has no way to pass a program its arguments",
                backend.name()
            ))
        })
    }
}

/// Which VM driver a program needs, read from the program itself, never declared by its author.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Driver {
    /// Straight-line: the synchronous VM.
    Plain,
    /// Tasks, pipes, `Select`: the cooperative scheduler.
    Scheduler,
    /// File I/O, sleep, networking, serving: the async host.
    Async,
}

/// The driver `source` needs, or why it does not parse.
pub fn driver_of(source: &str) -> Result<Driver, String> {
    logicaffeine_compile::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _types, _policies) = parsed?;
        Ok(
            if logicaffeine_compile::concurrency::uses_scheduler(stmts) {
                Driver::Scheduler
            } else if logicaffeine_compile::interpreter::needs_async(stmts, interner) {
                Driver::Async
            } else {
                Driver::Plain
            },
        )
    })
}

fn pinned_clock<T>(f: impl FnOnce() -> T) -> T {
    logicaffeine_compile::semantics::temporal::set_fixed_clock(FIXED_DAYS as i32, FIXED_NANOS);
    let out = f();
    logicaffeine_compile::semantics::temporal::clear_fixed_clock();
    out
}

/// Where this run's scratch files live: every compiled program runs with its working directory here. A worker's
/// lives in its job's directory, which the parent removes with the job, so a worker leaves nothing behind.
pub fn scratch_root() -> PathBuf {
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::var_os("LOGOS_PARITY_SCRATCH")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("LOGOS_PARITY_JOB").map(|job| PathBuf::from(job).join("scratch")))
            .unwrap_or_else(|| {
                std::env::temp_dir().join(format!("logos-parity-{}", std::process::id()))
            });
        std::fs::create_dir_all(&root).expect("the parity scratch directory");
        root
    })
    .clone()
}

/// A fresh directory for one (program, backend) run.
pub fn scratch_dir(label: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let safe: String = label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let dir = scratch_root().join(format!("{safe}-{n}"));
    std::fs::create_dir_all(&dir).expect("a run directory");
    dir
}

/// Whether `tool` runs on this machine.
pub fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn toolchain(tools: &[&str]) -> Option<Verdict> {
    let missing: Vec<&str> = tools.iter().copied().filter(|t| !have(t)).collect();
    (!missing.is_empty()).then(|| Verdict::Unavailable(format!("needs {}", missing.join(" + "))))
}

/// Run a compiled program under the fixed clock, a wall-clock limit and (unless `uncapped`) the
/// address-space cap, with its working directory at `dir`, reading both streams whole.
pub fn run_process(cmd: Command, dir: &Path, capped: bool) -> Verdict {
    run_process_capped(cmd, dir, capped.then_some(ADDRESS_SPACE_CAP))
}

/// [`run_process`] under an explicit address-space cap, or none.
pub fn run_process_capped(mut cmd: Command, dir: &Path, cap: Option<u64>) -> Verdict {
    if let Some(cap) = cap {
        let program = cmd.get_program().to_owned();
        let args: Vec<_> = cmd.get_args().map(|a| a.to_owned()).collect();
        let mut wrapped = Command::new("prlimit");
        wrapped
            .arg(format!("--as={cap}"))
            .arg("--")
            .arg(program)
            .args(args);
        for (k, v) in cmd.get_envs() {
            match v {
                Some(v) => {
                    wrapped.env(k, v);
                }
                None => {
                    wrapped.env_remove(k);
                }
            }
        }
        cmd = wrapped;
    }
    // The outermost run gets a process group of its own, and everything it starts (an isolated worker's
    // compilers, hosts and programs included) stays in it, so ending the group ends the whole run.
    let leads_a_group = std::env::var_os("LOGOS_PARITY_JOB").is_none();
    if leads_a_group {
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    }
    cmd.current_dir(dir)
        .env("LOGOS_FIXED_DAYS", FIXED_DAYS.to_string())
        .env("LOGOS_FIXED_NANOS", FIXED_NANOS.to_string())
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Verdict::Refused(format!("the program did not start: {e}")),
    };
    let mut out_pipe = child.stdout.take().expect("piped stdout");
    let mut err_pipe = child.stderr.take().expect("piped stderr");
    let out_reader = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = out_pipe.read_to_end(&mut v);
        v
    });
    let err_reader = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = err_pipe.read_to_end(&mut v);
        v
    });
    let pid = child.id();
    let kill_group = || {
        if leads_a_group {
            process_group::end_group(pid);
        }
    };
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() > RUN_LIMIT => {
                kill_group();
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(_) => break None,
        }
    };
    kill_group();
    let stdout = out_reader.join().unwrap_or_default();
    let stderr = err_reader.join().unwrap_or_default();
    match status {
        Some(status) => Verdict::Observed(Observation::of_process(status, stdout, stderr)),
        None => Verdict::Observed(Observation {
            stdout,
            stderr,
            end: End::Abnormal(format!("timed out after {} s", RUN_LIMIT.as_secs())),
        }),
    }
}

fn first_line(s: &str) -> String {
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .chars()
        .take(240)
        .collect()
}

/// Everything a refusal said, its lines joined: a tool that names its stage on one line and the reason on the next
/// is read whole.
fn said(s: &str) -> String {
    s.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(400)
        .collect()
}

/// The first error a compiler printed, or its first line.
fn first_error(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    text.lines()
        .find(|l| l.contains("error"))
        .map(|l| l.trim().chars().take(240).collect())
        .unwrap_or_else(|| first_line(&text))
}

/// Build with `cc` and run: the shared path of every backend whose output is a C-toolchain input.
fn build_and_run(
    build: Command,
    bin: &Path,
    run: Command,
    dir: &Path,
    capped: bool,
    what: &str,
) -> Verdict {
    let mut build = build;
    match build.current_dir(dir).output() {
        Ok(o) if o.status.success() && bin.exists() => run_process(run, dir, capped),
        Ok(o) => Verdict::Refused(format!(
            "the emitted {what} does not build: {}",
            first_error(&o.stderr)
        )),
        Err(e) => Verdict::Refused(format!("the {what} toolchain did not start: {e}")),
    }
}

/// Run `program` on `backend`. The Rust backends are built for the whole corpus at once ([`rust_batch`]) and
/// `built` carries those binaries; every other backend runs in a child process of this test binary
/// ([`run_isolated`]), so a hang, a stack overflow or a global a backend leaves set belongs to that one run.
pub fn run(backend: Backend, program: &Program, built: Option<&rust_batch::Built>) -> Verdict {
    match backend {
        Backend::RustDebug | Backend::RustRelease => run_direct(backend, program, built),
        _ => run_isolated(backend, program),
    }
}

/// The test every isolated run re-executes this binary into. It does nothing unless
/// `LOGOS_PARITY_JOB` names a job directory.
pub const WORKER_TEST: &str = "tier_parity_worker";

/// Why this test binary cannot run an isolated backend, if it cannot: a suite that declares no [`WORKER_TEST`]
/// re-executes into nothing, and every isolated run, the reference's included, would end without a verdict and
/// so agree with every other.
fn missing_worker() -> Option<String> {
    static LISTED: std::sync::OnceLock<Result<bool, String>> = std::sync::OnceLock::new();
    let listed = LISTED.get_or_init(|| {
        let exe = std::env::current_exe().map_err(|e| format!("this test binary: {e}"))?;
        let out = Command::new(&exe)
            .args(["--list", "--ignored"])
            .output()
            .map_err(|e| format!("listing {exe:?}: {e}"))?;
        let entry = format!("{WORKER_TEST}: test");
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .any(|l| l == entry))
    });
    match listed {
        Ok(true) => None,
        Ok(false) => Some(format!(
            "this suite declares no `{WORKER_TEST}`, so an isolated backend has nothing to run in; add\n\
             #[test]\n#[ignore = \"the worker the parity sweep re-executes this binary into\"]\n\
             fn {WORKER_TEST}() {{\n    tier_parity::worker_main();\n}}"
        )),
        Err(why) => Some(why.clone()),
    }
}

/// Run one job in a child process: the program and the backend's name go into a job directory, the child
/// runs [`worker_main`], and the verdict comes back through files in the same directory.
pub fn run_isolated(backend: Backend, program: &Program) -> Verdict {
    if let Some(why) = missing_worker() {
        panic!("{why}");
    }
    let job = scratch_dir(&format!("{}-{}-job", program.id, backend.name()));
    std::fs::write(job.join("program.lg"), &program.source).expect("write the job's program");
    std::fs::write(job.join("args"), program.args.join("\n")).expect("write the job's arguments");
    std::fs::write(job.join("id"), &program.id).expect("write the job's id");
    std::fs::write(job.join("backend"), backend.name()).expect("write the job's backend");
    let exe = std::env::current_exe().expect("this test binary");
    let mut cmd = Command::new(exe);
    cmd.args([
        WORKER_TEST,
        "--exact",
        "--ignored",
        "--nocapture",
        "--test-threads=1",
    ])
    .env("LOGOS_PARITY_JOB", &job)
    .env("RUST_MIN_STACK", (256u64 << 20).to_string());
    let verdict = match run_process(cmd, &job, false) {
        Verdict::Observed(child) => match read_verdict(&job) {
            Some(v) => v,
            None => Verdict::Observed(Observation {
                stdout: Vec::new(),
                stderr: child.stderr,
                end: End::Abnormal(match child.end {
                    End::Abnormal(why) => format!("the backend's process died: {why}"),
                    other => format!("the backend's process ended {other:?} without a verdict"),
                }),
            }),
        },
        other => other,
    };
    let _ = std::fs::remove_dir_all(&job);
    verdict
}

/// The child side of [`run_isolated`].
pub fn worker_main() {
    let Some(job) = std::env::var_os("LOGOS_PARITY_JOB").map(PathBuf::from) else {
        return;
    };
    let source = std::fs::read_to_string(job.join("program.lg")).expect("the job's program");
    let id = std::fs::read_to_string(job.join("id")).expect("the job's id");
    let name = std::fs::read_to_string(job.join("backend")).expect("the job's backend");
    let args: Vec<String> = std::fs::read_to_string(job.join("args"))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    let backend = Backend::ALL
        .into_iter()
        .find(|b| b.name() == name)
        .expect("a known backend");
    std::panic::set_hook(Box::new(|_| {}));
    let verdict = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_direct(
            backend,
            &Program {
                id: id.clone(),
                source: source.clone(),
                args: args.clone(),
            },
            None,
        )
    }))
    .unwrap_or_else(|panic| {
        Verdict::Observed(Observation {
            stdout: Vec::new(),
            stderr: panic_text(&*panic).into_bytes(),
            end: End::Abnormal("the backend panicked".to_string()),
        })
    });
    write_verdict(&job, &verdict);
}

fn write_verdict(job: &Path, v: &Verdict) {
    let (kind, text, obs) = match v {
        Verdict::Observed(o) => ("observed", String::new(), Some(o)),
        Verdict::Refused(s) => ("refused", s.clone(), None),
        Verdict::Unavailable(s) => ("unavailable", s.clone(), None),
        Verdict::Unrun(s) => ("unrun", s.clone(), None),
    };
    if let Some(o) = obs {
        std::fs::write(job.join("stdout.bin"), &o.stdout).expect("write stdout");
        std::fs::write(job.join("stderr.bin"), &o.stderr).expect("write stderr");
        let end = match &o.end {
            End::Finished => "finished".to_string(),
            End::Failed => "failed".to_string(),
            End::Abnormal(why) => format!("abnormal:{why}"),
        };
        std::fs::write(job.join("end"), end).expect("write the ending");
    }
    std::fs::write(job.join("text"), text).expect("write the verdict text");
    std::fs::write(job.join("verdict"), kind).expect("write the verdict");
}

fn read_verdict(job: &Path) -> Option<Verdict> {
    let kind = std::fs::read_to_string(job.join("verdict")).ok()?;
    let text = std::fs::read_to_string(job.join("text")).unwrap_or_default();
    Some(match kind.as_str() {
        "observed" => {
            let end = std::fs::read_to_string(job.join("end")).ok()?;
            Verdict::Observed(Observation {
                stdout: std::fs::read(job.join("stdout.bin")).ok()?,
                stderr: std::fs::read(job.join("stderr.bin")).ok()?,
                end: match end.as_str() {
                    "finished" => End::Finished,
                    "failed" => End::Failed,
                    other => {
                        End::Abnormal(other.strip_prefix("abnormal:").unwrap_or(other).to_string())
                    }
                },
            })
        }
        "refused" => Verdict::Refused(text),
        "unavailable" => Verdict::Unavailable(text),
        _ => Verdict::Unrun(text),
    })
}

/// Run `program` on `backend` in this process.
/// A program the front end rejects, as `largo` shows it on every backend: the report it prints before it
/// hands the program to any of them (`logicaffeine_cli::parse_error_report`, the file named as `largo`
/// names it from the project's root). `None` when the front end accepts the program.
pub fn rejection(source: &str) -> Option<Observation> {
    let pe = logicaffeine_compile::ui_bridge::with_parsed_program_checked(source, |parsed, _| {
        parsed.map(|_| ())
    })
    .err()?;
    let report = logicaffeine_cli::parse_error_report(Path::new("src/main.lg"), source, &pe);
    Some(Observation {
        stdout: Vec::new(),
        stderr: report.into_bytes(),
        end: End::Failed,
    })
}

pub fn run_direct(
    backend: Backend,
    program: &Program,
    built: Option<&rust_batch::Built>,
) -> Verdict {
    let src = program.source.as_str();
    let argv = program.argv();
    // `largo` rejects a program the front end rejects before any backend sees it, so every backend this
    // harness drives itself observes the report. The `largo` rows print it themselves, and a Rust batch
    // program is judged below by its own front end, which is a separate pipeline.
    if !matches!(
        backend,
        Backend::Interpret | Backend::RustDebug | Backend::RustRelease
    ) {
        if let Some(rejected) = rejection(src) {
            return Verdict::Observed(rejected);
        }
    }
    match backend {
        Backend::TreeWalker => pinned_clock(|| {
            let r = compile::tw_lines_with_args(src, &argv);
            Verdict::Observed(Observation::of_lines(&r.lines, r.error.as_deref()))
        }),
        Backend::Vm => pinned_clock(|| match driver_of(src) {
            Err(why) => Verdict::Observed(Observation::of_lines(&[], Some(&why))),
            Ok(Driver::Plain) => {
                let r = if program.args.is_empty() { compile::vm_outcome(src) } else { compile::vm_outcome_with_args(src, &argv, None) };
                Verdict::Observed(Observation::of_stream(r.output, r.error.as_deref()))
            }
            Ok(_) if !program.args.is_empty() => Verdict::Refused(
                "the VM's scheduler and async drivers have no way to pass a program its arguments".to_string(),
            ),
            Ok(Driver::Scheduler) => {
                let r = logicaffeine_compile::ui_bridge::run_vm_concurrent(src);
                Verdict::Observed(Observation::of_lines(&r.lines, r.error.as_deref()))
            }
            Ok(Driver::Async) => {
                let r = compile::vm_lines_net(src);
                Verdict::Observed(Observation::of_lines(&r.lines, r.error.as_deref()))
            }
        }),
        Backend::VmOptimized => pinned_clock(|| match driver_of(src) {
            Err(why) => Verdict::Observed(Observation::of_lines(&[], Some(&why))),
            Ok(Driver::Plain) => {
                let r = compile::vm_outcome_tiered(src, &argv, logicaffeine_language::optimization::Tier::T3, None);
                Verdict::Observed(Observation::of_stream(r.output, r.error.as_deref()))
            }
            Ok(Driver::Scheduler) if !program.args.is_empty() => Verdict::Refused(
                "the optimized scheduler run has no way to pass a program its arguments".to_string(),
            ),
            Ok(Driver::Scheduler) => match compile::compile_to_program_optimized(src) {
                Ok(p) => {
                    let r = compile::program_lines_concurrent(&p);
                    Verdict::Observed(Observation::of_lines(&r.lines, r.error.as_deref()))
                }
                Err(e) => Verdict::Refused(format!("the optimizer refused: {}", first_line(&format!("{e:?}")))),
            },
            Ok(Driver::Async) => Verdict::Unrun(
                "the product has no optimized VM entry for a program that needs the async host".to_string(),
            ),
        }),
        Backend::VmForge => pinned_clock(|| match driver_of(src) {
            Err(why) => Verdict::Observed(Observation::of_lines(&[], Some(&why))),
            Ok(Driver::Plain) => {
                let tier = logicaffeine_jit::ForgeTier::new();
                let r = compile::vm_outcome_with_args(src, &argv, Some(&tier as &dyn logicaffeine_compile::vm::NativeTier));
                Verdict::Observed(Observation::of_stream(r.output, r.error.as_deref()))
            }
            Ok(driver) => Verdict::Unrun(format!("the JIT is installed only on the synchronous VM; this program needs the {driver:?} driver")),
        }),
        Backend::Projection1 | Backend::Projection1OnVm | Backend::Projection1Optimizer => {
            if let Some(v) = program.needs_argv(backend) {
                return v;
            }
            let residual = match backend {
                Backend::Projection1 => compile::projection1_source_real_fast("", "", src),
                Backend::Projection1OnVm => compile::projection1_source_real_fast_on_vm("", "", src),
                _ => compile::projection1_source("", "", src),
            };
            match residual {
                Err(why) => Verdict::Refused(format!("the projection refused: {}", first_line(&why))),
                Ok(residual) => run_direct(Backend::Vm, &Program::new(program.id.clone(), residual), None),
            }
        }
        Backend::Projection2 | Backend::Projection3 => {
            if let Some(v) = program.needs_argv(backend) {
                return v;
            }
            let (types, interpreter) = (compile::core_types_for_pe_source(), compile::core_interp_source());
            let combined = if backend == Backend::Projection2 {
                compile::genuine_p2_on_target_source(src, types, interpreter)
            } else {
                compile::genuine_p3_on_target_source(src, types, interpreter)
            };
            match combined {
                Err(why) => Verdict::Refused(format!("the projection refused: {}", first_line(&why))),
                Ok(combined) => logicaffeine_compile::semantics::collections::with_reference_semantics(|| {
                    run_direct(Backend::Vm, &Program::new(program.id.clone(), combined), None)
                }),
            }
        }
        Backend::Interpret => {
            let dir = scratch_dir(&format!("{}-interpret", program.id));
            project(&dir, src);
            let mut cmd = Command::new(largo());
            cmd.args(["--quiet", "run", "--interpret"]).args(&program.args);
            let v = run_process(cmd, &dir, true);
            let _ = std::fs::remove_dir_all(&dir);
            v
        }
        Backend::RustDebug | Backend::RustRelease => {
            let Some(built) = built else {
                return Verdict::Unrun("the Rust backends run from the corpus-wide batch build".to_string());
            };
            if let Some(v) = toolchain(&["cargo"]) {
                return v;
            }
            let profile = if backend == Backend::RustDebug { rust_batch::Profile::Debug } else { rust_batch::Profile::Release };
            match built.binary(&program.id, profile) {
                // Both front ends rejected the program: the report. Only the Rust one: a difference.
                Err(why) if built.rejected(&program.id) => rejection(src).map(Verdict::Observed).unwrap_or(Verdict::Refused(why)),
                Err(why) => Verdict::Refused(why),
                Ok(bin) => {
                    let dir = scratch_dir(&format!("{}-{}", program.id, backend.name()));
                    let mut cmd = Command::new(bin);
                    cmd.args(&program.args);
                    let v = run_process(cmd, &dir, true);
                    let _ = std::fs::remove_dir_all(&dir);
                    v
                }
            }
        }
        Backend::C | Backend::CBytecode => {
            if let Some(v) = toolchain(&["gcc"]) {
                return v;
            }
            let emitted = if backend == Backend::C { compile::compile_to_c(src) } else { compile::compile_to_c_via_bytecode(src) };
            match emitted {
                Err(e) => Verdict::Refused(format!("refused: {}", first_line(&format!("{e:?}")))),
                Ok(code) => {
                    let links = match logicaffeine_compile::codegen_c::value_runtime::link_args(&code) {
                        Ok(links) => links,
                        Err(why) => return Verdict::Refused(format!("the value runtime did not build: {}", first_line(&why))),
                    };
                    let dir = scratch_dir(&format!("{}-{}", program.id, backend.name()));
                    std::fs::write(dir.join("prog.c"), code).expect("write C");
                    let mut gcc = Command::new("gcc");
                    gcc.args(["-O2", "-o", "prog", "prog.c"]).args(links).arg("-lm");
                    let mut prog = Command::new(dir.join("prog"));
                    prog.args(&program.args);
                    let v = build_and_run(gcc, &dir.join("prog"), prog, &dir, true, "C");
                    let _ = std::fs::remove_dir_all(&dir);
                    v
                }
            }
        }
        Backend::Wasm | Backend::WasmLinked => {
            if let Some(v) = toolchain(&["node"]) {
                return v;
            }
            let target = if backend == Backend::Wasm { "wasm" } else { "wasm-linked" };
            let dir = scratch_dir(&format!("{}-{target}", program.id));
            std::fs::write(dir.join("prog.lg"), src).expect("write the program");
            let emitted = Command::new(largo())
                .args(["--quiet", "emit", target, "prog.lg", "-o", "prog.wasm"])
                .current_dir(&dir)
                .env("NO_COLOR", "1")
                .output();
            let v = match emitted {
                Ok(o) if o.status.success() => {
                    let mut node = Command::new("node");
                    node.arg("prog.mjs").args(&program.args);
                    run_process(node, &dir, false)
                }
                Ok(o) => Verdict::Refused(format!("largo emit {target} refused: {}", said(&String::from_utf8_lossy(&o.stderr)))),
                Err(e) => Verdict::Refused(format!("largo did not start: {e}")),
            };
            let _ = std::fs::remove_dir_all(&dir);
            v
        }
        Backend::WasmDom => Verdict::Unrun(
            "a wasm-dom module runs in a browser host; this harness does not yet drive a headless browser per program".to_string(),
        ),
        Backend::RiscV => {
            if let Some(v) = toolchain(&["riscv64-linux-gnu-gcc", "qemu-riscv64"]) {
                return v;
            }
            match compile::compile_to_riscv(src) {
                Err(e) => Verdict::Refused(format!("refused: {}", first_line(&format!("{e:?}")))),
                Ok(asm) => {
                    let runtime = match logicaffeine_compile::codegen_riscv::runtime(&asm) {
                        Ok(runtime) => runtime,
                        Err(why) => return Verdict::Refused(format!("the value runtime did not build: {}", first_line(&why))),
                    };
                    let dir = scratch_dir(&format!("{}-riscv", program.id));
                    std::fs::write(dir.join("main.s"), asm).expect("write asm");
                    std::fs::write(dir.join("logos_rt.c"), runtime.c).expect("write the runtime");
                    let mut gcc = Command::new("riscv64-linux-gnu-gcc");
                    gcc.args(["-O2", "-static", "-o", "main", "main.s", "logos_rt.c"]).args(runtime.link_args);
                    let mut qemu = Command::new("qemu-riscv64");
                    qemu.arg(dir.join("main")).args(&program.args);
                    let v = match gcc.current_dir(&dir).output() {
                        Ok(o) if o.status.success() => run_process_capped(qemu, &dir, Some(QEMU_ADDRESS_SPACE_CAP)),
                        Ok(o) => Verdict::Refused(format!("the emitted RISC-V does not build: {}", first_error(&o.stderr))),
                        Err(e) => Verdict::Refused(format!("the RISC-V toolchain did not start: {e}")),
                    };
                    let _ = std::fs::remove_dir_all(&dir);
                    v
                }
            }
        }
        Backend::Cuda => {
            if let Some(v) = toolchain(&["nvcc"]) {
                return v;
            }
            if !Path::new("/dev/nvidia0").exists() {
                return Verdict::Unavailable("needs a GPU driver (/dev/nvidia0)".to_string());
            }
            match compile::compile_to_cuda(src) {
                Err(e) => Verdict::Refused(format!("refused: {}", first_line(&format!("{e:?}")))),
                Ok(art) => {
                    let dir = scratch_dir(&format!("{}-cuda", program.id));
                    std::fs::write(dir.join("prog.cu"), &art.kernel_cu).expect("write .cu");
                    let mut nvcc = Command::new("nvcc");
                    nvcc.args(["-O2", "-arch=sm_86", "-o", "prog", "prog.cu"]);
                    let mut prog = Command::new(dir.join("prog"));
                    prog.args(&program.args);
                    let v = build_and_run(nvcc, &dir.join("prog"), prog, &dir, false, "CUDA");
                    let _ = std::fs::remove_dir_all(&dir);
                    v
                }
            }
        }
        Backend::Tenstorrent => match compile::compile_to_tenstorrent(src) {
            Err(e) => Verdict::Refused(format!("refused: {}", first_line(&format!("{e:?}")))),
            Ok(art) => tenstorrent(&art, program),
        },
        Backend::Fabric => {
            if let Some(v) = program.needs_argv(backend) {
                return v;
            }
            use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{program_main_fabric, program_output};
            match std::panic::catch_unwind(|| program_output(&program_main_fabric(src), &[])) {
                Ok(lines) => Verdict::Observed(Observation::of_lines(&lines, None)),
                Err(panic) => Verdict::Refused(format!("the fabric refused: {}", first_line(&panic_text(&*panic)))),
            }
        }
        Backend::SoftCore => pinned_clock(|| {
            if let Some(v) = program.needs_argv(backend) {
                return v;
            }
            use logicaffeine_compile::codegen_sva::soft_core_model::{run_soft_core_outcome, ChipFailure, ShowVal};
            match logicaffeine_compile::codegen_sva::full_asm::compile_to_full_words(src) {
                Err(e) => Verdict::Refused(format!("refused: {}", first_line(&e))),
                Ok(words) => {
                    let run = run_soft_core_outcome(&words, 200_000_000);
                    let lines: Vec<String> = run.shows.iter().map(ShowVal::fmt).collect();
                    let failure = match run.failure {
                        None => None,
                        Some(ChipFailure::Reason(r)) => Some(r),
                        Some(ChipFailure::Trap) => Some(String::new()),
                    };
                    Verdict::Observed(Observation::of_lines(&lines, failure.as_deref()))
                }
            }
        }),
        Backend::Analog => match compile::compile_to_analog(src) {
            Err(e) => Verdict::Refused(format!("refused: {}", first_line(&format!("{e:?}")))),
            Ok(_) => Verdict::Unrun("this harness does not yet read a program's output back from the analog netlist's simulation".to_string()),
        },
        Backend::Plasticine => match compile::compile_to_plasticine(src) {
            Err(e) => Verdict::Refused(format!("refused: {}", first_line(&format!("{e:?}")))),
            Ok(_) => Verdict::Unrun("the Plasticine bundle runs under the Spatial toolchain, which this harness does not drive".to_string()),
        },
        Backend::Rdu => match compile::compile_to_rdu(src, logicaffeine_compile::spatial::rdu_chip::RduChip::Sn50) {
            Err(e) => Verdict::Refused(format!("refused: {}", first_line(&format!("{e:?}")))),
            Ok(_) => Verdict::Unrun("the RDU bundle runs on SambaNova hardware or its SDK, which this harness does not drive".to_string()),
        },
    }
}

/// What a panic said.
pub fn panic_text(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default()
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Run a TT-Metalium bundle on Tenstorrent's Wormhole simulator. The host program is the same for every
/// bundle, so it is built once per distinct host and CMake text; the kernel is JIT-compiled by tt-metal
/// from the working directory at run time. Every simulator run holds the repository's measurement lock
/// (`logs/tt/.measure.lock`), so no two sessions share the simulator.
fn tenstorrent(art: &logicaffeine_compile::codegen_tt::TtArtifact, program: &Program) -> Verdict {
    use std::hash::{Hash, Hasher};
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let tt_home = std::env::var_os("TT_METAL_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("tt-metal"));
    let sim = std::env::var_os("TT_METAL_SIMULATOR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("sim/libttsim_wh.so"));
    if !tt_home.join("build_Release/CMakeCache.txt").exists() || !sim.exists() {
        return Verdict::Unavailable(format!(
            "needs a built tt-metal ({}) and the ttsim simulator ({})",
            tt_home.display(),
            sim.display()
        ));
    }
    let cmake3 = Command::new("/usr/bin/cmake")
        .arg("--version")
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .next()
                .is_some_and(|l| l.contains(" 3."))
        })
        .unwrap_or(false);
    if !cmake3 {
        return Verdict::Unavailable(
            "needs cmake 3.x at /usr/bin/cmake (tt-metal does not build with 4.x)".to_string(),
        );
    }
    static HOSTS: std::sync::Mutex<BTreeMap<u64, Result<PathBuf, String>>> =
        std::sync::Mutex::new(BTreeMap::new());
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (&art.host_cpp, &art.cmake).hash(&mut hasher);
    let key = hasher.finish();
    let host = {
        let mut hosts = HOSTS.lock().unwrap();
        hosts
            .entry(key)
            .or_insert_with(|| {
                let dir = scratch_root().join(format!("tt-host-{key:016x}"));
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                std::fs::write(dir.join("logos_tt_host.cpp"), &art.host_cpp)
                    .map_err(|e| e.to_string())?;
                std::fs::write(dir.join("CMakeLists.txt"), &art.cmake)
                    .map_err(|e| e.to_string())?;
                std::fs::write(
                    dir.join(logicaffeine_compile::codegen_tt::KERNEL_FILE_NAME),
                    &art.kernel_cpp,
                )
                .map_err(|e| e.to_string())?;
                let path = format!("/usr/bin:{}", std::env::var("PATH").unwrap_or_default());
                for args in [&["-B", "build", "-S", "."][..], &["--build", "build"][..]] {
                    let o = Command::new("/usr/bin/cmake")
                        .args(args)
                        .current_dir(&dir)
                        .env("PATH", &path)
                        .env("TT_METAL_HOME", &tt_home)
                        .output()
                        .map_err(|e| format!("cmake did not start: {e}"))?;
                    if !o.status.success() {
                        return Err(format!(
                            "the TT-Metalium host does not build: {}",
                            first_error(&o.stderr)
                        ));
                    }
                }
                Ok(dir.join("build/logos_tt_host"))
            })
            .clone()
    };
    let host = match host {
        Ok(h) => h,
        Err(why) => return Verdict::Refused(why),
    };
    let dir = scratch_dir(&format!("{}-tenstorrent", program.id));
    std::fs::write(
        dir.join(logicaffeine_compile::codegen_tt::KERNEL_FILE_NAME),
        &art.kernel_cpp,
    )
    .expect("write the kernel");
    let lock = repo_root().join("logs/tt/.measure.lock");
    let _ = std::fs::create_dir_all(lock.parent().expect("the lock's directory"));
    let mut cmd = Command::new("flock");
    cmd.arg(&lock)
        .arg(&host)
        .args(&program.args)
        .env("TT_METAL_HOME", &tt_home)
        .env("TT_METAL_SIMULATOR", &sim)
        .env("TT_METAL_SLOW_DISPATCH_MODE", "1")
        .env("TT_METAL_DISABLE_SFPLOADMACRO", "1")
        .env("TT_METAL_DPRINT_CORES", "0,0");
    let v = run_process(cmd, &dir, false);
    let _ = std::fs::remove_dir_all(&dir);
    v
}

#[path = "../support/under_test.rs"]
mod under_test;

/// The shipped `largo`, found where the run put it.
pub fn largo() -> PathBuf {
    under_test::largo()
}

/// A one-file project in `dir` whose entry is `source`.
pub fn project(dir: &Path, source: &str) {
    std::fs::create_dir_all(dir.join("src")).expect("a project directory");
    std::fs::write(
        dir.join("Largo.toml"),
        "[package]\nname = \"parity\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n",
    )
    .expect("write Largo.toml");
    std::fs::write(dir.join("src/main.lg"), source).expect("write the entry");
}

/// One census row: a program on a backend, judged against the reference backend.
#[derive(Clone, Debug)]
pub struct Row {
    pub program: String,
    pub backend: Backend,
    pub verdict: Verdict,
    /// `None` when the backend observed exactly what the reference observed.
    pub difference: Option<String>,
}

impl Row {
    pub fn matches(&self) -> bool {
        matches!(self.verdict, Verdict::Observed(_)) && self.difference.is_none()
    }

    pub fn status(&self) -> &'static str {
        match (&self.verdict, &self.difference) {
            (Verdict::Observed(_), None) => "MATCH",
            (Verdict::Observed(_), Some(_)) => "DIVERGED",
            (v, _) => v.label(),
        }
    }

    pub fn detail(&self) -> String {
        match (&self.verdict, &self.difference) {
            (Verdict::Observed(_), Some(d)) => d.clone(),
            (Verdict::Observed(_), None) => String::new(),
            (Verdict::Refused(s) | Verdict::Unavailable(s) | Verdict::Unrun(s), _) => s.clone(),
        }
    }
}

/// The backend every other backend is held to.
pub const REFERENCE: Backend = Backend::Vm;

/// Every verdict of a sweep, by (program id, backend).
pub type Matrix = BTreeMap<(String, Backend), Verdict>;

/// Run every program on every backend in `backends`, `jobs` at a time.
pub fn run_matrix(
    programs: &[Program],
    backends: &[Backend],
    jobs: usize,
    built: Option<&rust_batch::Built>,
) -> Matrix {
    let work: Vec<(usize, Backend)> = (0..programs.len())
        .flat_map(|p| backends.iter().copied().map(move |b| (p, b)))
        .collect();
    let next = AtomicUsize::new(0);
    let results: std::sync::Mutex<Matrix> = Default::default();
    std::thread::scope(|scope| {
        for _ in 0..jobs.max(1) {
            std::thread::Builder::new()
                .stack_size(256 << 20)
                .spawn_scoped(scope, || loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&(p, b)) = work.get(i) else { break };
                    let program = &programs[p];
                    let verdict = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        run(b, program, built)
                    }))
                    .unwrap_or_else(|panic| {
                        Verdict::Observed(Observation {
                            stdout: Vec::new(),
                            stderr: panic_text(&*panic).into_bytes(),
                            end: End::Abnormal("the backend panicked".to_string()),
                        })
                    });
                    results
                        .lock()
                        .unwrap()
                        .insert((program.id.clone(), b), verdict);
                })
                .expect("a worker thread");
        }
    });
    results.into_inner().unwrap()
}

/// Judge every (program, backend) verdict of `results` against [`REFERENCE`]'s verdict for the same program,
/// and the reference itself against the tree-walker when both ran, so the two in-process engines are held to
/// each other as well.
pub fn judge(programs: &[Program], backends: &[Backend], results: &Matrix) -> Vec<Row> {
    let mut rows = Vec::new();
    for program in programs {
        let id = program.id.clone();
        let reference = results.get(&(id.clone(), REFERENCE));
        for &b in backends {
            let Some(verdict) = results.get(&(id.clone(), b)).cloned() else {
                continue;
            };
            let against = if b == REFERENCE {
                results.get(&(id.clone(), Backend::TreeWalker))
            } else {
                reference
            };
            let difference = match (&verdict, against) {
                (Verdict::Observed(got), Some(Verdict::Observed(want))) => {
                    if b == REFERENCE {
                        want.difference(got).map(|d| format!("the tree-walker {d}"))
                    } else {
                        got.difference(want)
                    }
                }
                (Verdict::Observed(_), Some(other)) => {
                    Some(format!("the reference did not run: {other:?}"))
                }
                (Verdict::Observed(_), None) if b == REFERENCE => None,
                (Verdict::Observed(_), None) => Some("the reference was not run".to_string()),
                _ => None,
            };
            rows.push(Row {
                program: id.clone(),
                backend: b,
                verdict,
                difference,
            });
        }
    }
    rows
}

/// How many programs one Rust batch builds, runs and deletes before the next, so the disk the Rust columns
/// take is bounded by one chunk however large the corpus grows.
pub const RUST_CHUNK: usize = 150;

/// Run every program on every backend and judge the lot. The reference always runs; the Rust backends run a
/// chunk of the corpus at a time from a batch build that is deleted before the next chunk is built.
pub fn sweep(programs: &[Program], backends: &[Backend], jobs: usize) -> Result<Vec<Row>, String> {
    if let Some(why) = missing_worker() {
        return Err(why);
    }
    warm_value_runtimes(backends);
    let is_rust = |b: &Backend| matches!(b, Backend::RustDebug | Backend::RustRelease);
    let mut in_process: Vec<Backend> = vec![REFERENCE];
    in_process.extend(
        backends
            .iter()
            .copied()
            .filter(|b| *b != REFERENCE && !is_rust(b)),
    );
    let rust: Vec<Backend> = backends.iter().copied().filter(is_rust).collect();
    let mut results = run_matrix(programs, &in_process, jobs, None);
    if !rust.is_empty() {
        for chunk in programs.chunks(RUST_CHUNK) {
            let built = rust_batch::build(chunk, jobs)?;
            results.extend(run_matrix(chunk, &rust, jobs, Some(&built)));
            built.clean();
        }
        rust_batch::release();
    }
    Ok(judge(programs, backends, &results))
}

/// Build every runtime a swept backend links once, untimed, before any program is timed: linked WebAssembly's
/// runtime, and the value runtime's archive for the host (both C backends) and for RISC-V. The first build of one,
/// on a box whose temporary directory starts empty or after the value model changes, takes longer than one
/// program's run limit, and every run after it reuses the build. A failure is left for the runs to report.
fn warm_value_runtimes(backends: &[Backend]) {
    use logicaffeine_compile::codegen_c::value_runtime::{archive_for, Target};
    if backends.contains(&Backend::WasmLinked) {
        #[cfg(feature = "wasm-jit")]
        let _ = logicaffeine_compile::compile::compile_to_wasm_linked("## Main\nShow 1.\n");
    }
    if backends.iter().any(|b| matches!(b, Backend::C | Backend::CBytecode)) {
        let _ = archive_for(Target::Host);
    }
    if backends.contains(&Backend::RiscV) {
        let _ = archive_for(Target::Riscv64);
    }
}

/// The census as tab-separated values: one row per (program, backend), with the verdict and its detail.
pub fn census_tsv(rows: &[Row]) -> String {
    let mut out = String::from("program\tbackend\tstatus\tdetail\n");
    for r in rows {
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\n",
            r.program,
            r.backend.name(),
            r.status(),
            r.detail().replace(['\t', '\n'], " ")
        ));
    }
    out
}

/// Per backend: how many programs matched, diverged, were refused, were unavailable, or went unrun.
pub fn census_summary(rows: &[Row]) -> String {
    let mut by: BTreeMap<Backend, BTreeMap<&'static str, usize>> = BTreeMap::new();
    for r in rows {
        *by.entry(r.backend)
            .or_default()
            .entry(r.status())
            .or_default() += 1;
    }
    let mut out = String::from("backend\tMATCH\tDIVERGED\tREFUSED\tUNAVAILABLE\tUNRUN\n");
    for (b, counts) in by {
        let n = |k: &str| counts.get(k).copied().unwrap_or(0);
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\n",
            b.name(),
            n("MATCH"),
            n("DIVERGED"),
            n("REFUSED"),
            n("UNAVAILABLE"),
            n("UNRUN")
        ));
    }
    out
}
