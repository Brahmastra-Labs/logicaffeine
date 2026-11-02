//! The value runtime a C program links when the exact extension reaches it: `logicaffeine-data`'s C ABI
//! over the compiled value model (`logos_rt_value_*`, see `logicaffeine_data::abi`), built on demand as
//! one static library, so both C backends compute with the compiler's own values. A program the
//! extension never reaches declares nothing and links nothing more than it did.
//!
//! The library is a two-line crate over `logicaffeine-data` with its `c-abi` feature, built in release
//! under a scratch directory of its own: path-depending on this checkout's `logicaffeine_data` inside
//! the monorepo, on the crates.io release of this compiler's own version anywhere else. Each build's
//! archive is copied to a file named by its content, which is never written again, so a link always
//! reads a whole archive however many processes build at once. It is built for the machine a program
//! runs on ([`Target`]): the host for the C backends, RV64GC Linux for the RISC-V tier.

use std::path::{Path, PathBuf};

/// What an emitted C program that calls the value runtime declares, and what [`links`] reads.
pub(crate) const DECLARATION: &str = "#define LOGOS_LINKS_VALUE_RUNTIME 1";

/// Whether the emitted C program `code` links the value runtime.
pub fn links(code: &str) -> bool {
    code.contains(DECLARATION)
}

/// What `code` puts on its link line after its own source: the value runtime's archive and the system
/// libraries the archive calls, when the program links it; nothing otherwise.
pub fn link_args(code: &str) -> Result<Vec<String>, String> {
    if !links(code) {
        return Ok(Vec::new());
    }
    archive_link_args(Target::Host)
}

/// The archive built for `target` and the system libraries it calls, as a link line names them.
pub fn archive_link_args(target: Target) -> Result<Vec<String>, String> {
    let archive = archive_for(target)?;
    let mut args = vec![archive.path.display().to_string()];
    args.extend(archive.system_libs.iter().cloned());
    Ok(args)
}

/// The machine a library is built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    /// The machine the compiler runs on, where the C backends' programs run.
    Host,
    /// RV64GC Linux with glibc, which the RISC-V tier links its programs for, statically.
    Riscv64,
}

impl Target {
    /// The triple cargo builds for; `None` for the host.
    pub fn triple(self) -> Option<&'static str> {
        match self {
            Target::Host => None,
            Target::Riscv64 => Some("riscv64gc-unknown-linux-gnu"),
        }
    }

    /// The file the system libraries of the target's last build are kept in.
    fn system_libs_file(self) -> String {
        match self.triple() {
            None => SYSTEM_LIBS.to_string(),
            Some(triple) => format!("system-libs-{triple}.txt"),
        }
    }

    /// The system libraries rustc names for the archive, as the target's link spells them: a static link has no
    /// shared unwinder, so libgcc_s is the static libgcc_eh there.
    fn link_libs(self, libs: Vec<String>) -> Vec<String> {
        match self {
            Target::Host => libs,
            Target::Riscv64 => libs
                .into_iter()
                .map(|l| if l == "-lgcc_s" { "-lgcc_eh".to_string() } else { l })
                .collect(),
        }
    }
}

/// The archive cargo writes for `target` under the target directory `target_dir`.
fn built_archive(target_dir: &Path, target: Target) -> PathBuf {
    match target.triple() {
        None => target_dir.join("release").join(ARCHIVE),
        Some(triple) => target_dir.join(triple).join("release").join(ARCHIVE),
    }
}

/// The built library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Archive {
    /// The archive, named by its content and never written again.
    pub path: PathBuf,
    /// The system libraries a link line names after the archive (`-lpthread`, `-ldl`, …).
    pub system_libs: Vec<String>,
}

/// The library's root: the ABI, and the allocator whose count the program's collector reads.
const ROOT: &str = "//! The C value runtime: `logicaffeine-data`'s C ABI and the allocator that counts what it holds.\n\
pub use logicaffeine_data::abi::*;\n\
\n\
#[global_allocator]\n\
static ALLOCATOR: logicaffeine_data::abi::Counting = logicaffeine_data::abi::Counting;\n";

/// The archive cargo writes for the library's crate.
const ARCHIVE: &str = "liblogos_value_runtime.a";

/// The system libraries of the last build, kept beside the archive for a build cargo found fresh.
const SYSTEM_LIBS: &str = "system-libs.txt";

/// The library's manifest, its `logicaffeine-data` dependency as `data` spells it.
fn manifest(data: &str) -> String {
    format!(
        "[package]\nname = \"logos-value-runtime\"\nversion = \"0.0.0\"\nedition = \"2021\"\npublish = false\n\n\
         [lib]\npath = \"lib.rs\"\ncrate-type = [\"staticlib\"]\n\n\
         [dependencies]\n{data}\n\n\
         [profile.release]\ndebug = false\n\n\
         [workspace]\n"
    )
}

/// The `logicaffeine-data` dependency line: this checkout's crate inside the monorepo, the crates.io
/// release of this compiler's version anywhere else.
fn data_dependency() -> String {
    match crate::compile::find_workspace_root() {
        Ok(root) => format!(
            "logicaffeine-data = {{ path = {:?}, features = [\"c-abi\"] }}",
            root.join("crates/logicaffeine_data").display().to_string()
        ),
        Err(_) => format!(
            "logicaffeine-data = {{ version = \"={}\", features = [\"c-abi\"] }}",
            env!("CARGO_PKG_VERSION")
        ),
    }
}

/// The system libraries a `native-static-libs` note names, in its order.
fn system_libs_of(note: &str) -> Option<Vec<String>> {
    let (_, libs) = note.split_once("native-static-libs:")?;
    Some(libs.split_whitespace().map(str::to_string).collect())
}

/// FNV-1a over the archive's bytes: the name of the file its content is kept in.
fn content_name(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("logos_value_runtime-{hash:016x}.a")
}

/// Write `contents` to `path` unless it already holds exactly them, so cargo sees an unchanged source
/// as unchanged.
fn write_if_changed(path: &Path, contents: &str) -> Result<(), String> {
    if std::fs::read_to_string(path).is_ok_and(|have| have == contents) {
        return Ok(());
    }
    std::fs::write(path, contents).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// The library for the host, built once per process.
pub fn archive() -> Result<std::sync::Arc<Archive>, String> {
    archive_for(Target::Host)
}

/// The library for `target`, built once per process.
#[cfg(not(target_arch = "wasm32"))]
pub fn archive_for(target: Target) -> Result<std::sync::Arc<Archive>, String> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    static BUILT: OnceLock<Mutex<HashMap<Target, Arc<Archive>>>> = OnceLock::new();
    let mut built = BUILT
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(archive) = built.get(&target) {
        return Ok(Arc::clone(archive));
    }
    let archive = Arc::new(build(target)?);
    built.insert(target, Arc::clone(&archive));
    Ok(archive)
}

#[cfg(target_arch = "wasm32")]
pub fn archive_for(_target: Target) -> Result<std::sync::Arc<Archive>, String> {
    Err("the C value runtime is built by a native toolchain".to_string())
}

/// The scratch directory the library builds in: one per checkout of the compiler, so two source trees
/// never share a build.
#[cfg(not(target_arch = "wasm32"))]
fn project_dir() -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    env!("CARGO_MANIFEST_DIR").hash(&mut hash);
    std::env::temp_dir().join(format!("logos_value_runtime-{:016x}", hash.finish()))
}

#[cfg(not(target_arch = "wasm32"))]
fn build(target: Target) -> Result<Archive, String> {
    use std::process::Command;
    let project = project_dir();
    std::fs::create_dir_all(project.join("archives"))
        .map_err(|e| format!("cannot create {}: {e}", project.display()))?;
    let lock_path = project.with_extension("lock");
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|e| format!("cannot open {}: {e}", lock_path.display()))?;
    fs2::FileExt::lock_exclusive(&lock).map_err(|e| format!("cannot lock {}: {e}", lock_path.display()))?;

    write_if_changed(&project.join("Cargo.toml"), &manifest(&data_dependency()))?;
    write_if_changed(&project.join("lib.rs"), ROOT)?;
    if let Ok(root) = crate::compile::find_workspace_root() {
        let seed = root.join("Cargo.lock");
        let lockfile = project.join("Cargo.lock");
        if seed.exists() && !lockfile.exists() {
            std::fs::copy(&seed, &lockfile).map_err(|e| format!("cannot seed {}: {e}", lockfile.display()))?;
        }
    }

    let target_dir = project.join("target");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut command = Command::new(cargo);
    command
        .args(["rustc", "--release", "--lib", "--quiet", "--manifest-path"])
        .arg(project.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(&target_dir);
    if let Some(triple) = target.triple() {
        command.args(["--target", triple]);
    }
    let out = command
        .args(["--", "--print", "native-static-libs"])
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("CARGO_BUILD_RUSTFLAGS")
        .env_remove("CARGO_TARGET_DIR")
        .output()
        .map_err(|e| format!("cargo did not start: {e}"))?;
    let said = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        let install = target
            .triple()
            .map(|triple| format!(" (its standard library installs with `rustup target add {triple}`)"))
            .unwrap_or_default();
        return Err(format!("cargo could not build the C value runtime{install}:\n{said}"));
    }
    let libs_path = project.join(target.system_libs_file());
    let system_libs = match said.lines().find_map(system_libs_of) {
        Some(libs) => {
            write_if_changed(&libs_path, &libs.join(" "))?;
            libs
        }
        None => std::fs::read_to_string(&libs_path)
            .map(|kept| kept.split_whitespace().map(str::to_string).collect())
            .map_err(|_| "cargo built the C value runtime without naming the system libraries it calls".to_string())?,
    };

    let built = built_archive(&target_dir, target);
    let bytes = std::fs::read(&built).map_err(|e| format!("cannot read {}: {e}", built.display()))?;
    let path = project.join("archives").join(content_name(&bytes));
    if !path.exists() {
        let partial = path.with_extension(format!("a.{}", std::process::id()));
        std::fs::write(&partial, &bytes).map_err(|e| format!("cannot write {}: {e}", partial.display()))?;
        std::fs::rename(&partial, &path).map_err(|e| format!("cannot keep {}: {e}", path.display()))?;
    }
    drop(lock);
    Ok(Archive {
        path,
        system_libs: target.link_libs(system_libs),
    })
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_c/value_runtime/tests.rs"]
mod tests;
