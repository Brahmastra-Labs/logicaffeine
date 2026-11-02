//! The in-test ttsim harness — runs a compiler-emitted Tensix program under Tenstorrent's simulator THROUGH
//! the repo's single measurement path (`scripts/tt-matmul-measure.sh` → `scripts/tt-gemm-ledger.py`), so
//! every cycle number a spatial lock asserts on is a ledger row parsed from a real run log, never typed.
//!
//! One simulator, one build tree: every run takes an exclusive file lock (`logs/tt/.measure.lock`) so two
//! sessions can never interleave a build/run or race the ledger write.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// The user's home directory, or an empty path when `HOME` is unset — then the default trees below are relative
/// paths nothing is built at, and [`available`] is false unless `TT_METAL_HOME` / `TT_METAL_SIMULATOR` name them.
fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

pub fn tt_metal_home() -> PathBuf {
    std::env::var_os("TT_METAL_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join("tt-metal"))
}

pub fn sim_so() -> PathBuf {
    std::env::var_os("TT_METAL_SIMULATOR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join("sim/libttsim_wh.so"))
}

/// The simulator is usable: a built tt-metal tree (its CMake cache), the Wormhole simulator object, the SOC
/// descriptor, the measurement script, and cmake 3.x on `/usr/bin` (4.x breaks the tt-metal build).
pub fn available() -> bool {
    let home = tt_metal_home();
    let cmake_ok = Command::new("/usr/bin/cmake")
        .arg("--version")
        .output()
        .map(|o| {
            o.status.success()
                && String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .next()
                    .map(|l| l.contains(" 3."))
                    .unwrap_or(false)
        })
        .unwrap_or(false);
    home.join("build_Release/CMakeCache.txt").exists()
        && home
            .join("tt_metal/soc_descriptors/wormhole_b0_80_arch.yaml")
            .exists()
        && sim_so().exists()
        && repo_root().join("scripts/tt-matmul-measure.sh").exists()
        && repo_root().join("scripts/tt-gemm-ledger.py").exists()
        && cmake_ok
}

/// The compute knobs a program is measured at (the ledger's matched-numerics key).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Knobs {
    pub fidelity: &'static str,
    pub fp32: bool,
    pub l1acc: bool,
}

impl Knobs {
    pub fn ledger_key(&self) -> String {
        format!(
            "{}-fp32_{}-l1acc_{}",
            self.fidelity, self.fp32 as u8, self.l1acc as u8
        )
    }
}

/// Exclusive lock around one measurement (build + simulate + ledger write). Held until dropped.
pub struct MeasureLock {
    _file: std::fs::File,
}

pub fn lock_measurements() -> MeasureLock {
    let dir = repo_root().join("logs/tt");
    std::fs::create_dir_all(&dir).expect("logs/tt");
    let path = dir.join(".measure.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&path)
        .expect("open the measure lock");
    let started = std::time::Instant::now();
    loop {
        // flock(2) through the shell keeps this dependency-free; the lock lives with the file handle we hold.
        let status = Command::new("flock")
            .arg("--nonblock")
            .arg(&path)
            .arg("true")
            .status();
        match status {
            Ok(s) if s.success() => break,
            _ => {
                assert!(
                    started.elapsed() < std::time::Duration::from_secs(3600),
                    "another measurement has held the ttsim lock for an hour"
                );
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
        }
    }
    MeasureLock { _file: file }
}

/// The committed GEMM ledger.
pub const COMMITTED_LEDGER: &str = "benchmarks/silicon/tenstorrent_gemm.json";

static RECORDING: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// The ledger this process's measurements record into. A test run records into its own copy of the committed
/// ledger: a directory under `logs/tt/ledgers/`, seeded once per process with the ledger and every log excerpt
/// its rows cite (the recorder re-verifies each row against its excerpt, resolved beside the ledger, and writes
/// the new run's excerpt there too), so running a test never writes into the tree. A lock reads the rows it
/// measured back from [`recorded`]. Recording into the committed ledger is an explicit act:
/// `LOGOS_RECORD_TT_LEDGER=1` on the run, or `scripts/tt-matmul-measure.sh` by hand.
pub fn recording_ledger() -> PathBuf {
    RECORDING
        .get_or_init(|| {
            let committed = repo_root().join(COMMITTED_LEDGER);
            if std::env::var_os("LOGOS_RECORD_TT_LEDGER").is_some_and(|v| v == "1") {
                return committed;
            }
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let dir = repo_root()
                .join("logs/tt/ledgers")
                .join(format!("{}-{stamp}", std::process::id()));
            seed_recording_dir(&committed, &dir);
            dir.join(committed.file_name().expect("the ledger is a file"))
        })
        .clone()
}

/// Copy `committed` and every excerpt its rows cite (each row's `log`, relative to the ledger) into `dir`.
fn seed_recording_dir(committed: &Path, dir: &Path) {
    let ledger_dir = committed.parent().expect("the ledger has a directory");
    let text = std::fs::read_to_string(committed)
        .unwrap_or_else(|e| panic!("read {}: {e}", committed.display()));
    let ledger: serde_json::Value =
        serde_json::from_str(&text).expect("the committed ledger is JSON");
    for row in ledger["rows"].as_array().expect("the ledger has rows") {
        let Some(log) = row["log"].as_str() else {
            continue;
        };
        let (from, to) = (ledger_dir.join(log), dir.join(log));
        std::fs::create_dir_all(to.parent().expect("an excerpt has a directory"))
            .expect("the recording excerpt directory");
        std::fs::copy(&from, &to)
            .unwrap_or_else(|e| panic!("seed the excerpt {}: {e}", from.display()));
    }
    std::fs::write(
        dir.join(committed.file_name().expect("the ledger is a file")),
        text,
    )
    .expect("seed the recording ledger");
}

/// The rows a lock reads: the recording ledger once this process has measured, the committed ledger until then.
pub fn recorded() -> serde_json::Value {
    let path = RECORDING
        .get()
        .cloned()
        .unwrap_or_else(|| repo_root().join(COMMITTED_LEDGER));
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).expect("the recording ledger is JSON")
}

/// Run `scripts/tt-matmul-measure.sh spatial …` for one shape at one knob set; returns the measure script's
/// stdout+stderr on success. The row it recorded lands in [`recording_ledger`], read back through [`recorded`].
pub fn measure_spatial(
    shape: (usize, usize, usize),
    chip: &str,
    knobs: Knobs,
) -> Result<String, String> {
    let _guard = lock_measurements();
    let script = repo_root().join("scripts/tt-matmul-measure.sh");
    let out = Command::new(&script)
        .current_dir(repo_root())
        .env("TT_METAL_HOME", tt_metal_home())
        .env("LEDGER", recording_ledger())
        .args([
            "spatial",
            "--shape",
            &shape.0.to_string(),
            &shape.1.to_string(),
            &shape.2.to_string(),
            "--chip",
            chip,
            "--fidelity",
            knobs.fidelity,
            "--fp32",
            &(knobs.fp32 as u8).to_string(),
            "--l1acc",
            &(knobs.l1acc as u8).to_string(),
        ])
        .output()
        .map_err(|e| format!("cannot run the measure script: {e}"))?;
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if out.status.success() {
        Ok(text)
    } else {
        Err(format!(
            "tt-matmul-measure.sh spatial {shape:?} failed ({}):\n{text}",
            out.status
        ))
    }
}

/// Sha256 of the emitted program the way the ledger hashes `--program-src` (host file + every kernel file,
/// sorted by path, concatenated) — so a lock can prove a ledger row was recorded from THIS compiler output.
pub fn program_sha256(files: &[(String, String)]) -> String {
    // The ledger is handed `host.cpp` and the `kernels` DIRECTORY, so every name is relative to its own
    // root: `host.cpp` and the bare kernel file names.
    let mut named: Vec<(String, &String)> = files
        .iter()
        .map(|(path, body)| (path.rsplit('/').next().unwrap_or(path).to_string(), body))
        .collect();
    named.sort_by(|a, b| a.0.cmp(&b.0));
    let mut h = Sha256::new();
    for (name, body) in named {
        h.update(name.as_bytes());
        h.update(b"\0");
        h.update(body.as_bytes());
        h.update(b"\0");
    }
    h.hex()
}

/// A dependency-free SHA-256 (the locks may not pull a crypto crate; the ledger tool's Python does the same).
pub struct Sha256 {
    state: [u32; 8],
    buf: Vec<u8>,
    len: u64,
}

impl Sha256 {
    pub fn new() -> Self {
        Sha256 {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buf: Vec::new(),
            len: 0,
        }
    }
    pub fn update(&mut self, data: &[u8]) {
        self.len += data.len() as u64;
        self.buf.extend_from_slice(data);
        while self.buf.len() >= 64 {
            let block: [u8; 64] = self.buf[..64].try_into().unwrap();
            self.compress(&block);
            self.buf.drain(..64);
        }
    }
    fn compress(&mut self, block: &[u8; 64]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[4 * i],
                block[4 * i + 1],
                block[4 * i + 2],
                block[4 * i + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (s, v) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *s = s.wrapping_add(v);
        }
    }
    pub fn hex(mut self) -> String {
        let bit_len = self.len * 8;
        let mut pad = vec![0x80u8];
        while (self.buf.len() + pad.len()) % 64 != 56 {
            pad.push(0);
        }
        pad.extend_from_slice(&bit_len.to_be_bytes());
        let tail = std::mem::take(&mut self.buf);
        let mut all = tail;
        all.extend_from_slice(&pad);
        for chunk in all.chunks(64) {
            let block: [u8; 64] = chunk.try_into().unwrap();
            self.compress(&block);
        }
        self.state.iter().map(|w| format!("{w:08x}")).collect()
    }
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}
