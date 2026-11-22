//! The sealed test world.
//!
//! While a `## Test` runs, a thread-local [`TestWorld`] intercepts the ambient
//! IO natives with SIMULATED resources that have real semantics: a virtual
//! clock frozen at a fixed epoch, an in-memory filesystem seeded by
//! `Given a file …`, an empty environment, and a per-test seeded RNG. There is
//! no unrecorded entropy API to misuse — a test is a pure function of
//! `(program, seed)`. Capability sentences (`This test may use the real
//! filesystem.`) opt individual resources back to reality under `--live`.
//!
//! Zero cost outside tests: each native pays ONE thread-local check on an IO
//! call (noise next to the syscall it guards); compute paths pay nothing.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};

/// The sealed world's fixed epoch: 2020-01-01T00:00:00Z in ms — a stable,
/// obviously-synthetic date (no test can accidentally depend on today).
pub const SEALED_EPOCH_MS: u64 = 1_577_836_800_000;

/// Which resources a test may touch FOR REAL (granted by capability sentences).
#[derive(Debug, Clone, Copy, Default)]
pub struct Capabilities {
    pub filesystem: bool,
    pub network: bool,
    pub environment: bool,
    pub clock: bool,
    /// `This test may use the real processes.` — `runProcess` starts real programs.
    pub processes: bool,
}

/// One test's sealed world.
#[derive(Debug)]
pub struct TestWorld {
    pub caps: Capabilities,
    /// Virtual clock (ms since the Unix epoch), starting at [`SEALED_EPOCH_MS`].
    pub clock_ms: u64,
    /// The in-memory filesystem (path → content), empty per test.
    pub files: HashMap<String, String>,
    /// Directories `makeDirs` created in the sealed filesystem. A directory also exists implicitly
    /// wherever a file lies beneath it.
    pub dirs: BTreeSet<String>,
    /// The sealed environment, empty unless the test declares variables.
    pub env: HashMap<String, String>,
    /// SplitMix64 state — the test's recorded entropy stream.
    pub rng: u64,
}

impl TestWorld {
    /// A fresh sealed world under the test's identity-keyed seed.
    pub fn new(caps: Capabilities, seed: u64) -> Self {
        TestWorld {
            caps,
            clock_ms: SEALED_EPOCH_MS,
            files: HashMap::new(),
            dirs: BTreeSet::new(),
            env: HashMap::new(),
            rng: seed,
        }
    }

    fn next_u64(&mut self) -> u64 {
        // SplitMix64 — identical to the runtime's seeded stream discipline.
        self.rng = self.rng.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

thread_local! {
    static WORLD: RefCell<Option<TestWorld>> = const { RefCell::new(None) };
}

/// Install a sealed world for the current thread (the runner, per test).
pub fn install(world: TestWorld) {
    WORLD.with(|w| *w.borrow_mut() = Some(world));
}

/// Remove the sealed world (the runner, after each test).
pub fn uninstall() {
    WORLD.with(|w| *w.borrow_mut() = None);
}

/// Take the installed world, if any, leaving none installed — a nested caller (a candidate page
/// tier compared against the reference) saves whatever world its caller had before installing its
/// own, and [`restore`]s it when it is done.
pub fn take() -> Option<TestWorld> {
    WORLD.with(|w| w.borrow_mut().take())
}

/// Put back a world a matching [`take`] returned (or `None`, mirroring its absence).
pub fn restore(world: Option<TestWorld>) {
    WORLD.with(|w| *w.borrow_mut() = world);
}

/// Run `f` against the installed world, if any.
pub fn with_world<R>(f: impl FnOnce(&mut TestWorld) -> R) -> Option<R> {
    WORLD.with(|w| w.borrow_mut().as_mut().map(f))
}

/// Seed a fixture file (`Given a file "…" containing "…".`).
pub fn seed_file(path: String, content: String) {
    with_world(|w| {
        w.files.insert(path, content);
    });
}

/// Advance the virtual clock (`Advance time by …`). Returns false when no
/// world is installed (the caller reports the test-only error).
pub fn advance_clock(ms: u64) -> bool {
    with_world(|w| w.clock_ms = w.clock_ms.saturating_add(ms)).is_some()
}

// ── the native intercepts (None = no sealed world / capability granted → real) ──

pub fn intercept_read(path: &str) -> Option<Result<String, String>> {
    with_world(|w| {
        if w.caps.filesystem {
            return None;
        }
        Some(match w.files.get(path) {
            Some(content) => Ok(content.clone()),
            None => Err(format!(
                "'{path}' is not in the test's sealed filesystem — seed it with \
                 `Given a file \"{path}\" containing \"…\".`, or grant \
                 `This test may use the real filesystem.`"
            )),
        })
    })
    .flatten()
}

pub fn intercept_write(path: &str, content: &str) -> Option<Result<(), String>> {
    with_world(|w| {
        if w.caps.filesystem {
            return None;
        }
        w.files.insert(path.to_string(), content.to_string());
        Some(Ok(()))
    })
    .flatten()
}

/// A sealed path, compared as written but without a trailing `/` (the root keeps its own).
fn sealed_path(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() && path.starts_with('/') {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Whether `dir` is a directory of the sealed filesystem: made by `makeDirs`, or above a file.
fn sealed_is_dir(w: &TestWorld, dir: &str) -> bool {
    let prefix = if dir == "/" { "/".to_string() } else { format!("{dir}/") };
    w.dirs.contains(dir)
        || w.files.keys().any(|f| f.starts_with(&prefix))
        || w.dirs.iter().any(|d| d.starts_with(&prefix))
}

pub fn intercept_append(path: &str, content: &str) -> Option<Result<(), String>> {
    with_world(|w| {
        if w.caps.filesystem {
            return None;
        }
        w.files.entry(sealed_path(path)).or_default().push_str(content);
        Some(Ok(()))
    })
    .flatten()
}

pub fn intercept_exists(path: &str) -> Option<bool> {
    with_world(|w| {
        if w.caps.filesystem {
            return None;
        }
        let p = sealed_path(path);
        Some(w.files.contains_key(&p) || sealed_is_dir(w, &p))
    })
    .flatten()
}

pub fn intercept_list_dir(path: &str) -> Option<Result<Vec<String>, String>> {
    with_world(|w| {
        if w.caps.filesystem {
            return None;
        }
        let dir = sealed_path(path);
        if !sealed_is_dir(w, &dir) {
            return Some(Err(format!(
                "'{path}' is not a directory in the test's sealed filesystem — create it with \
                 `makeDirs`, seed a file beneath it, or grant `This test may use the real filesystem.`"
            )));
        }
        let prefix = if dir == "/" { "/".to_string() } else { format!("{dir}/") };
        let mut names = BTreeSet::new();
        for entry in w.files.keys().chain(w.dirs.iter()) {
            if let Some(rest) = entry.strip_prefix(&prefix) {
                if let Some(first) = rest.split('/').next().filter(|n| !n.is_empty()) {
                    names.insert(first.to_string());
                }
            }
        }
        Some(Ok(names.into_iter().collect()))
    })
    .flatten()
}

pub fn intercept_make_dirs(path: &str) -> Option<Result<(), String>> {
    with_world(|w| {
        if w.caps.filesystem {
            return None;
        }
        let dir = sealed_path(path);
        if w.files.contains_key(&dir) {
            return Some(Err(format!(
                "Failed to make directories '{path}': a file is already there"
            )));
        }
        let absolute = dir.starts_with('/');
        let mut at = String::new();
        for part in dir.split('/').filter(|p| !p.is_empty()) {
            if absolute || !at.is_empty() {
                at.push('/');
            }
            at.push_str(part);
            w.dirs.insert(at.clone());
        }
        Some(Ok(()))
    })
    .flatten()
}

pub fn intercept_rename(from: &str, to: &str) -> Option<Result<(), String>> {
    with_world(|w| {
        if w.caps.filesystem {
            return None;
        }
        match w.files.remove(&sealed_path(from)) {
            Some(content) => {
                w.files.insert(sealed_path(to), content);
                Some(Ok(()))
            }
            None => Some(Err(format!(
                "Failed to rename '{from}' to '{to}': '{from}' is not in the test's sealed filesystem"
            ))),
        }
    })
    .flatten()
}

pub fn intercept_env_get(key: &str) -> Option<Option<String>> {
    with_world(|w| {
        if w.caps.environment {
            return None;
        }
        Some(w.env.get(key).cloned())
    })
    .flatten()
}

/// What `runProcess` answers inside a sealed test that was not granted real processes — the same
/// sentence on every engine.
pub const PROCESS_SEALED: &str = "runProcess: a sealed test starts no processes — grant `This test may use the real processes.` and run under `largo test --live`";

/// A sealed world starts no processes: `Some(refusal)` while a test runs without the
/// `This test may use the real processes.` grant, `None` (run for real) otherwise.
pub fn intercept_process() -> Option<String> {
    with_world(|w| {
        if w.caps.processes {
            return None;
        }
        Some(PROCESS_SEALED.to_string())
    })
    .flatten()
}

pub fn intercept_now() -> Option<u64> {
    with_world(|w| if w.caps.clock { None } else { Some(w.clock_ms) }).flatten()
}

/// A sealed `sleep` advances logical time INSTANTLY — the statement that turns
/// a timeout test into a microsecond assertion. Returns true when handled.
pub fn intercept_sleep(ms: u64) -> bool {
    with_world(|w| {
        if w.caps.clock {
            return false;
        }
        w.clock_ms = w.clock_ms.saturating_add(ms);
        true
    })
    .unwrap_or(false)
}

/// A uniform draw from `min..=max` off the world's seeded stream: a word at or above the largest
/// multiple of the span is drawn again, so no value is likelier than another — the rule every real
/// host's `randomInt` draws by.
pub fn intercept_random_int(min: i64, max: i64) -> Option<i64> {
    with_world(|w| {
        if max <= min {
            return min;
        }
        let span = max.wrapping_sub(min) as u64 as u128 + 1;
        if span == 1 << 64 {
            return w.next_u64() as i64;
        }
        let span = span as u64;
        let biased_tail = (u64::MAX % span + 1) % span;
        loop {
            let r = w.next_u64();
            if r <= u64::MAX - biased_tail {
                return min.wrapping_add((r % span) as i64);
            }
        }
    })
}

pub fn intercept_random_float() -> Option<f64> {
    with_world(|w| (w.next_u64() >> 11) as f64 / (1u64 << 53) as f64)
}
