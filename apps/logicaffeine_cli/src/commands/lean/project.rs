//! Project mode: compile a Lean project in isolation with its profile's toolchain, establish the
//! target's assumptions with Lean's own `#print axioms`, and export the target with the profile's
//! pinned lean4export.
//!
//! The project's `lean-toolchain` selects its [`Profile`] (absent, Stage A); a pin outside the
//! registry is `unsupported_toolchain` before Lean runs. The Lean that runs must report the
//! profile's release and commit, and the exporter must be the profile's by content — every source
//! by SHA-256 and, for an executable profile, the built executable by the digest pinned for this
//! platform (or, on a platform without a pin, by the digest its build receipt records). The
//! exporter is `--exporter`, else (Stage A) `$LOGOS_LEAN4EXPORT` or `~/.logos/lean/lean4export`,
//! else the registry entry `<registry>/<version>/`, the registry being `$LOGOS_LEAN_EXPORTERS` or
//! `~/.logos/lean/exporters` (`scripts/lean/install-lean-exporters.py` builds it).
//!
//! Nothing here trusts an exit code alone. Lean compiles a `sorry` with exit code 0 and a warning,
//! and the exporter PANICS on an unknown constant and still exits 0; so the target is resolved and
//! its axioms read from `#print axioms` before the exporter runs, and the exporter's output is held
//! to its own contract (the intake, under the same profile) afterwards.
//!
//! Deterministic resolution. A project of Lean core only: its modules are its `.lean` files; an
//! import is a project module or a Lean core module (`Init`, `Std`, `Lean`), resolved before Lean
//! runs, and every module is compiled in an isolated copy, in dependency order. A project with Lake
//! dependencies (Mathlib): every `lake-manifest.json` dependency must be a git package checked out
//! at exactly its pinned revision and built; only the target's `--module` is compiled — by the
//! profile's Lake in the project's own context, without building or writing anything there, its
//! imports the project's prebuilt build — into the isolated work directory, and the run records
//! every dependency's identity and carries the project's configuration with the module's source.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use logicaffeine_kernel::interface::lean::intake::{self, Component};
use logicaffeine_kernel::interface::lean::pipeline::{Diagnostic, LeanEvidence, LeanResult};
use logicaffeine_kernel::interface::lean::profile::{self, ExporterForm, Profile};
use logicaffeine_kernel::interface::lean::DiagnosticCode;

use super::process::{self, Failure};

pub(crate) struct Options {
    pub lean: PathBuf,
    /// `--exporter`: the profile's exporter, overriding every default location.
    pub exporter: Option<PathBuf>,
    /// `--module`: the module declaring the target, in a project with Lake dependencies.
    pub module: Option<String>,
    /// `--statement`: Lean also builds the target's statement artifact, and `#print axioms` is
    /// Lean's report for that artifact rather than for the target's proof.
    pub statement: bool,
    pub timeout_secs: u64,
    pub max_export_bytes: u64,
}

pub(crate) struct Exported {
    pub export: String,
    pub evidence: LeanEvidence,
    /// The profile the project was produced under.
    pub profile: &'static Profile,
}

type Refusal = Box<LeanResult>;

/// The Lean-core packages a project may import without a Lake dependency.
const CORE: [&str; 3] = ["Init", "Std", "Lean"];

/// The longest output kept from a Lean compilation or audit.
const LEAN_OUTPUT_CAP: u64 = 16 << 20;

fn refuse(target: &str, stage: &'static str, diagnostic: Diagnostic) -> Refusal {
    let mut result = LeanResult::new(target);
    if stage != "lean_accepted" {
        result.pass("lean_accepted", "Lean compiled the project in isolation");
    }
    result.fail(stage, diagnostic);
    Box::new(result)
}

fn lean_refusal(target: &str, code: DiagnosticCode, message: impl Into<String>) -> Refusal {
    refuse(
        target,
        "lean_accepted",
        Diagnostic::new("lean_accepted", code, message),
    )
}

fn deadline_refusal(target: &str, stage: &'static str, options: &Options, doing: &str) -> Refusal {
    refuse(
        target,
        stage,
        Diagnostic::new(
            stage,
            DiagnosticCode::ResourceLimit,
            format!(
                "the declared deadline of {} s passed before {doing} finished",
                options.timeout_secs
            ),
        ),
    )
}

/// One `.lean` file of the project.
struct Module {
    /// The module name, as Lean prints it.
    name: String,
    /// The file, relative to the project root.
    relative: PathBuf,
    /// `(imported module, line)`.
    imports: Vec<(String, usize)>,
}

/// A scratch directory removed when the run ends, however it ends.
struct WorkDir(PathBuf);

impl WorkDir {
    fn new() -> std::io::Result<Self> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("logos-lean-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&path)?;
        Ok(WorkDir(path))
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(crate) fn export(
    project: &Path,
    target: &str,
    options: &Options,
    deadline: Instant,
) -> Result<Exported, Refusal> {
    if Instant::now() >= deadline {
        return Err(deadline_refusal(
            target,
            "lean_accepted",
            options,
            "Lean could run",
        ));
    }
    let profile = select_profile(project, target)?;
    let lake = lake_dependencies(project, target)?;
    let core = match &lake {
        Some(_) => None,
        None => {
            let modules = discover(project, target)?;
            let order = resolve(&modules, target)?;
            Some((modules, order))
        }
    };

    // The preflight reports every missing piece of the pinned toolchain at once.
    // A Lean given as a path runs from the isolated work directory too: make it absolute.
    let lean = &if options.lean.components().count() > 1 {
        options
            .lean
            .canonicalize()
            .unwrap_or_else(|_| options.lean.clone())
    } else {
        options.lean.clone()
    };
    let toolchain = check_toolchain(lean, profile, target, options, deadline);
    let exporter = locate_exporter(options.exporter.as_deref(), profile)
        .and_then(|dir| check_exporter(&dir, profile));
    let ((prefix, toolchain_identity), exporter) = match (toolchain, exporter) {
        (Ok(toolchain), Ok(exporter)) => (toolchain, exporter),
        (toolchain, exporter) => {
            if let Err(refusal) = &toolchain {
                if refusal
                    .diagnostics
                    .iter()
                    .any(|d| d.code == DiagnosticCode::ResourceLimit)
                {
                    return Err(toolchain.err().expect("a refusal"));
                }
            }
            let mut result = LeanResult::new(target);
            for problem in toolchain
                .err()
                .map(|r| r.diagnostics)
                .unwrap_or_default()
                .into_iter()
                .chain(exporter.err())
            {
                result.fail("lean_accepted", problem);
            }
            return Err(Box::new(result));
        }
    };
    let work = WorkDir::new().map_err(|e| {
        lean_refusal(
            target,
            DiagnosticCode::Infrastructure,
            format!("cannot create an isolated work directory: {e}"),
        )
    })?;
    let environment = match (core, lake) {
        (Some((modules, order)), _) => {
            check_core_imports(&modules, &prefix, profile, target)?;
            compile_core_project(
                project, &modules, &order, &work, lean, profile, target, options, deadline,
            )?
        }
        (None, Some(lake)) => compile_lake_module(
            project, &lake, &work, &prefix, profile, target, options, deadline,
        )?,
        (None, None) => unreachable!("a project is a core project or a Lake project"),
    };

    let axioms = if options.statement {
        statement_axioms(&environment, target, &work.0, profile, options, deadline)?
    } else {
        print_axioms(&environment, target, &work.0, profile, options, deadline)?
    };

    let search = join_paths(&environment.search);
    let mut command = match profile.exporter.form {
        ExporterForm::Interpreted => {
            let mut command = Command::new(lean);
            let mut path = vec![exporter.join(".lake/build/lib")];
            path.extend(environment.search.iter().cloned());
            command
                .arg("--run")
                .arg(exporter.join("Main.lean"))
                .arg("--")
                .env("LEAN_PATH", join_paths(&path));
            command
        }
        ExporterForm::Executable => {
            let mut command = Command::new(exporter.join(EXECUTABLE));
            command
                .env("LEAN_SYSROOT", &prefix)
                .env("LEAN_PATH", &search);
            command
        }
    };
    command
        .args(profile.exporter.flags)
        .args(&environment.imports)
        .arg("--")
        .arg(printed(target))
        .current_dir(&work.0)
        .env("ELAN_TOOLCHAIN", profile.toolchain)
        .env_remove("LEAN_SRC_PATH");
    let exported =
        process::run(command, deadline, options.max_export_bytes).map_err(|f| match f {
            Failure::Deadline => {
                deadline_refusal(target, "translated", options, "the profile's exporter")
            }
            Failure::Spawn(e) => refuse(
                target,
                "translated",
                Diagnostic::new(
                    "translated",
                    DiagnosticCode::Infrastructure,
                    format!("cannot run the profile's exporter: {e}"),
                ),
            ),
        })?;
    if exported.stdout_truncated {
        return Err(refuse(
            target,
            "translated",
            Diagnostic::new(
                "translated",
                DiagnosticCode::ResourceLimit,
                format!(
                    "the export exceeds the declared budget of {} bytes",
                    options.max_export_bytes
                ),
            ),
        ));
    }
    let stderr = String::from_utf8_lossy(&exported.stderr);
    if !exported.status.success()
        || stderr.contains("PANIC")
        || stderr.contains("uncaught exception")
    {
        return Err(refuse(
            target,
            "translated",
            Diagnostic::new(
                "translated",
                DiagnosticCode::Infrastructure,
                format!(
                    "the profile's exporter failed ({}): {}",
                    exported.status,
                    stderr.trim()
                ),
            ),
        ));
    }
    let export = String::from_utf8(exported.stdout).map_err(|_| {
        refuse(
            target,
            "translated",
            Diagnostic::new(
                "translated",
                DiagnosticCode::Infrastructure,
                "the profile's exporter wrote output that is not UTF-8",
            ),
        )
    })?;
    // The export must name exactly the profile this run selected, ran and exported with: a header
    // naming another release than the toolchain that produced it is refused here, before intake.
    if let Err(verdict) = intake::read_header(export.lines().next().unwrap_or_default(), profile) {
        return Err(refuse(
            target,
            "translated",
            Diagnostic::of_refusal("translated", &verdict).with_line(1),
        ));
    }
    Ok(Exported {
        export,
        evidence: LeanEvidence::Accepted {
            axioms,
            sources: environment.sources,
            toolchain: toolchain_identity,
            exporter: profile.exporter_identity(&host_platform()),
            dependencies: environment.dependencies,
        },
        profile,
    })
}

/// What a compiled project offers Lean afterwards: the modules an audit and the export import, the
/// search path that resolves them (the isolated work directory first), the sources the package
/// carries, and the identities of the prebuilt Lake dependencies the compilation reused.
struct Environment {
    imports: Vec<String>,
    search: Vec<PathBuf>,
    sources: BTreeMap<String, Vec<u8>>,
    dependencies: Vec<String>,
}

/// A project of Lean core only: every module is copied into the work directory and compiled there,
/// in dependency order.
#[allow(clippy::too_many_arguments)]
fn compile_core_project(
    project: &Path,
    modules: &[Module],
    order: &[usize],
    work: &WorkDir,
    lean: &Path,
    profile: &Profile,
    target: &str,
    options: &Options,
    deadline: Instant,
) -> Result<Environment, Refusal> {
    for module in modules {
        let destination = work.0.join(&module.relative);
        let copied = destination
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|_| std::fs::copy(project.join(&module.relative), &destination).map(|_| ()));
        copied.map_err(|e| {
            lean_refusal(
                target,
                DiagnosticCode::Infrastructure,
                format!("cannot isolate {}: {e}", module.relative.display()),
            )
        })?;
    }
    for &index in order {
        let module = &modules[index];
        let olean = module.relative.with_extension("olean");
        let mut command = Command::new(lean);
        command
            .arg("-o")
            .arg(&olean)
            .arg(&module.relative)
            .current_dir(&work.0)
            .env("LEAN_PATH", &work.0);
        let compiled = run_lean(command, profile, deadline).map_err(|f| {
            tool_failure(
                f,
                target,
                options,
                lean,
                &format!("compiling {}", module.relative.display()),
            )
        })?;
        if !compiled.status.success() {
            return Err(lean_rejection(target, &module.relative, &compiled));
        }
    }
    let mut sources = BTreeMap::new();
    for module in modules {
        let bytes = std::fs::read(work.0.join(&module.relative)).map_err(|e| {
            lean_refusal(
                target,
                DiagnosticCode::Infrastructure,
                format!("cannot read {}: {e}", module.relative.display()),
            )
        })?;
        sources.insert(slash_path(&module.relative), bytes);
    }
    Ok(Environment {
        imports: modules.iter().map(|m| m.name.clone()).collect(),
        search: vec![work.0.clone()],
        sources,
        dependencies: Vec::new(),
    })
}

/// A project-relative path with `/` separators, as a package names it.
fn slash_path(relative: &Path) -> String {
    relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// The built executable of an executable profile, relative to its exporter directory.
const EXECUTABLE: &str = ".lake/build/bin/lean4export";

/// The platform triple the exporter digests are pinned for.
pub(crate) fn host_platform() -> String {
    let arch = std::env::consts::ARCH;
    match std::env::consts::OS {
        "linux" => format!("{arch}-unknown-linux-gnu"),
        "macos" => format!("{arch}-apple-darwin"),
        "windows" => format!("{arch}-pc-windows-msvc"),
        os => format!("{arch}-{os}"),
    }
}

/// Run the profile's Lean with its toolchain selected and no inherited search path.
fn run_lean(
    mut command: Command,
    profile: &Profile,
    deadline: Instant,
) -> Result<process::Captured, Failure> {
    command
        .env("ELAN_TOOLCHAIN", profile.toolchain)
        .env_remove("LEAN_SRC_PATH");
    if command.get_envs().all(|(key, _)| key != "LEAN_PATH") {
        command.env_remove("LEAN_PATH");
    }
    process::run(command, deadline, LEAN_OUTPUT_CAP)
}

fn lean_command(lean: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(lean);
    command.args(args);
    command
}

fn tool_failure(
    failure: Failure,
    target: &str,
    options: &Options,
    lean: &Path,
    doing: &str,
) -> Refusal {
    match failure {
        Failure::Deadline => {
            deadline_refusal(target, "lean_accepted", options, &format!("Lean ({doing})"))
        }
        Failure::Spawn(e) => lean_refusal(
            target,
            DiagnosticCode::Infrastructure,
            format!(
                "cannot run the Lean toolchain {} ({doing}): {e}",
                lean.display()
            ),
        ),
    }
}

fn join_paths(paths: &[PathBuf]) -> std::ffi::OsString {
    std::env::join_paths(paths).unwrap_or_default()
}

/// The target as Lean reads it back (components that are not identifiers in `«…»`).
fn printed(target: &str) -> String {
    intake::parse_lean_name(target)
        .map(|c| intake::print_lean_name(&c))
        .unwrap_or_else(|_| target.to_string())
}

/// The profile a project's `lean-toolchain` pins; a project without one is Stage A.
fn select_profile(project: &Path, target: &str) -> Result<&'static Profile, Refusal> {
    let Ok(pin) = std::fs::read_to_string(project.join("lean-toolchain")) else {
        return Ok(&profile::STAGE_A);
    };
    profile::for_toolchain(&pin).map_err(|verdict| {
        lean_refusal(
            target,
            DiagnosticCode::UnsupportedToolchain,
            verdict.reason().message(),
        )
    })
}

/// The project's configuration files, carried with its sources: they pin the toolchain and every
/// Lake dependency the compilation reused.
const CONFIGURATION: [&str; 4] = [
    "lean-toolchain",
    "lakefile.toml",
    "lakefile.lean",
    "lake-manifest.json",
];

/// A project with Lake dependencies, its pins verified against what is checked out.
struct LakeProject {
    /// One identity line per dependency: name, the pinned revision, the requested revision, the URL.
    dependencies: Vec<String>,
}

/// The project's Lake dependencies, when it has any, each verified: a git dependency pinned by a
/// full revision in `lake-manifest.json`, checked out at exactly that revision (read from the
/// checkout's own `HEAD`, never trusted from its directory name). A project whose lakefile requires
/// packages it never resolved, a dependency of another kind or a checkout at another revision is
/// refused before Lean runs. Whether what the target's module imports is built, and up to date with
/// its sources, is Lake's own `--no-build` check when the module is compiled.
fn lake_dependencies(project: &Path, target: &str) -> Result<Option<LakeProject>, Refusal> {
    let requires = ["lakefile.lean", "lakefile.toml"].into_iter().any(|file| {
        std::fs::read_to_string(project.join(file)).is_ok_and(|text| {
            text.lines().any(|l| {
                let l = l.trim_start();
                l.starts_with("require ") || l.starts_with("[[require]]")
            })
        })
    });
    let manifest = match std::fs::read_to_string(project.join("lake-manifest.json")) {
        Ok(text) => Some(
            serde_json::from_str::<serde_json::Value>(&text).map_err(|e| {
                lean_refusal(
                    target,
                    DiagnosticCode::MalformedExport,
                    format!("lake-manifest.json is not JSON: {e}"),
                )
            })?,
        ),
        Err(_) => None,
    };
    let packages = manifest
        .as_ref()
        .and_then(|m| m["packages"].as_array().cloned())
        .unwrap_or_default();
    if packages.is_empty() {
        if requires {
            return Err(lean_refusal(
                target,
                DiagnosticCode::Infrastructure,
                "the lakefile requires Lake packages that lake-manifest.json does not pin: resolve them with `lake update` and build them first",
            ));
        }
        return Ok(None);
    }
    let manifest = manifest.expect("packages came from a manifest");
    let packages_dir = project.join(manifest["packagesDir"].as_str().unwrap_or(".lake/packages"));
    let mut dependencies = Vec::new();
    for package in &packages {
        let name = package["name"].as_str().unwrap_or_default();
        let kind = package["type"].as_str().unwrap_or_default();
        let refused = |code: DiagnosticCode, message: String| lean_refusal(target, code, message);
        if name.is_empty() || name.contains(['/', '\\']) || name.starts_with('.') {
            return Err(refused(
                DiagnosticCode::MalformedExport,
                format!(
                    "lake-manifest.json names a dependency {name:?}, which is not a package name"
                ),
            ));
        }
        if kind != "git" {
            return Err(refused(
                DiagnosticCode::UnsupportedFeature,
                format!("the Lake dependency {name} is a {kind:?} dependency; a result reuses only git dependencies pinned by a revision"),
            ));
        }
        let rev = package["rev"].as_str().unwrap_or_default();
        if rev.len() != 40 || !rev.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(refused(
                DiagnosticCode::MalformedExport,
                format!(
                    "lake-manifest.json pins {name} at {rev:?}, which is not a full git revision"
                ),
            ));
        }
        let checkout = packages_dir.join(name);
        let head = git_head(&checkout).map_err(|why| {
            refused(
                DiagnosticCode::Infrastructure,
                format!(
                    "the Lake dependency {name} is not checked out at {}: {why}",
                    checkout.display()
                ),
            )
        })?;
        if !head.eq_ignore_ascii_case(rev) {
            return Err(refused(
                DiagnosticCode::Infrastructure,
                format!("the checked-out {name} is at {head}, but lake-manifest.json pins {rev}: the project's dependencies are not the ones it declares (run `lake update` or check out the pinned revision)"),
            ));
        }
        dependencies.push(format!(
            "{name} {rev} ({}) {}",
            package["inputRev"].as_str().unwrap_or("unpinned"),
            package["url"].as_str().unwrap_or_default()
        ));
    }
    Ok(Some(LakeProject { dependencies }))
}

/// The revision a git checkout's `HEAD` names, read from its files: a detached `HEAD` holds the
/// revision, a symbolic one names a ref resolved through the loose refs, then `packed-refs`.
fn git_head(checkout: &Path) -> Result<String, String> {
    let mut git = checkout.join(".git");
    if git.is_file() {
        let pointer = std::fs::read_to_string(&git).map_err(|e| e.to_string())?;
        let dir = pointer
            .trim()
            .strip_prefix("gitdir:")
            .ok_or("its .git file names no git directory")?
            .trim();
        git = checkout.join(dir);
    }
    let head = std::fs::read_to_string(git.join("HEAD")).map_err(|e| format!("no HEAD ({e})"))?;
    let head = head.trim();
    let Some(reference) = head.strip_prefix("ref:").map(str::trim) else {
        return Ok(head.to_string());
    };
    if let Ok(loose) = std::fs::read_to_string(git.join(reference)) {
        return Ok(loose.trim().to_string());
    }
    let packed = std::fs::read_to_string(git.join("packed-refs")).map_err(|e| e.to_string())?;
    packed
        .lines()
        .filter(|l| !l.starts_with('#') && !l.starts_with('^'))
        .find_map(|l| {
            let (rev, name) = l.split_once(' ')?;
            (name.trim() == reference).then(|| rev.to_string())
        })
        .ok_or_else(|| format!("HEAD names {reference}, which is not a ref of the checkout"))
}

/// Overlay, in `work`, the build root that holds the package of the module at `relative` (a path
/// without extension): the directories down to the module are real, every other entry of them a
/// symbolic link to the prebuilt build, and the module's own artifacts are left out, for the fresh
/// compilation to write.
fn overlay(work: &Path, search: &[PathBuf], relative: &Path) -> std::io::Result<()> {
    let mut parts: Vec<std::ffi::OsString> = relative
        .components()
        .map(|c| c.as_os_str().to_os_string())
        .collect();
    let stem = parts.pop().unwrap_or_default();
    let Some(package) = parts.first() else {
        return std::fs::create_dir_all(work);
    };
    let Some(root) = search.iter().find(|root| root.join(package).is_dir()) else {
        return std::fs::create_dir_all(work.join(parts.iter().collect::<PathBuf>()));
    };
    let mut built = root.clone();
    let mut overlaid = work.to_path_buf();
    for (depth, part) in parts.iter().enumerate() {
        built.push(part);
        overlaid.push(part);
        std::fs::create_dir_all(&overlaid)?;
        let next = parts.get(depth + 1);
        let artifact = format!("{}.", stem.to_string_lossy());
        for entry in std::fs::read_dir(&built)? {
            let entry = entry?;
            let name = entry.file_name();
            let is_next = next.is_some_and(|n| *n == name);
            let is_target = next.is_none() && name.to_string_lossy().starts_with(&artifact);
            if is_next || is_target {
                continue;
            }
            let link = overlaid.join(&name);
            if !link.exists() {
                symlink(&entry.path(), &link)?;
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn symlink(original: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(original, link)
}

#[cfg(windows)]
fn symlink(original: &Path, link: &Path) -> std::io::Result<()> {
    if original.is_dir() {
        std::os::windows::fs::symlink_dir(original, link)
    } else {
        std::os::windows::fs::symlink_file(original, link)
    }
}

/// Whether the project has Lake dependencies, so that `--module` must name the target's module.
pub(crate) fn has_lake_dependencies(project: &Path) -> bool {
    std::fs::read_to_string(project.join("lake-manifest.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v["packages"].as_array().map(|p| !p.is_empty()))
        .unwrap_or(false)
}

/// A project with Lake dependencies: only the target's module is compiled, from the project's own
/// source, into the isolated work directory, by the profile's Lake in the project's own context
/// (`lake --no-build -H lean`: the lakefile's options and library rules, every import resolved to
/// the project's prebuilt build and its pinned dependencies, every file re-hashed rather than its
/// `.hash` trusted, and nothing built or written in the project). The module's source is read
/// before and after compilation, and must not change.
#[allow(clippy::too_many_arguments)]
fn compile_lake_module(
    project: &Path,
    lake: &LakeProject,
    work: &WorkDir,
    prefix: &Path,
    profile: &Profile,
    target: &str,
    options: &Options,
    deadline: Instant,
) -> Result<Environment, Refusal> {
    let Some(module) = options.module.as_deref() else {
        return Err(lean_refusal(
            target,
            DiagnosticCode::Usage,
            "--module <Module> must name the module declaring the target in a project with Lake dependencies",
        ));
    };
    let components = intake::parse_lean_name(module).map_err(|verdict| {
        lean_refusal(target, DiagnosticCode::Usage, verdict.reason().message())
    })?;
    let mut relative = PathBuf::new();
    for component in &components {
        match component {
            Component::Str(part) => relative.push(part),
            Component::Num(n) => relative.push(n.to_string()),
        }
    }
    let source_file = relative.with_extension("lean");
    let source = std::fs::read(project.join(&source_file)).map_err(|e| {
        lean_refusal(
            target,
            DiagnosticCode::MissingModule,
            format!(
                "the project has no module {module}: cannot read {}: {e}",
                source_file.display()
            ),
        )
    })?;
    let lake_binary = prefix.join("bin/lake");
    let olean = work.0.join(relative.with_extension("olean"));
    if let Some(parent) = olean.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            lean_refusal(
                target,
                DiagnosticCode::Infrastructure,
                format!("cannot prepare the work directory: {e}"),
            )
        })?;
    }
    let lake_command = |args: &[&std::ffi::OsStr]| {
        let mut command = Command::new(&lake_binary);
        command
            .arg("--no-build")
            .arg("-H")
            .args(args)
            .current_dir(project)
            .env(
                "PATH",
                join_paths(&[
                    prefix.join("bin"),
                    PathBuf::from("/usr/bin"),
                    PathBuf::from("/bin"),
                ]),
            )
            .env_remove("LEAN_PATH")
            .env_remove("LAKE")
            .env_remove("LAKE_HOME");
        command
    };
    let printed_path = run_lean(
        lake_command(&["env".as_ref(), "printenv".as_ref(), "LEAN_PATH".as_ref()]),
        profile,
        deadline,
    )
    .map_err(|f| tool_failure(f, target, options, &lake_binary, "reporting its LEAN_PATH"))?;
    if !printed_path.status.success() {
        return Err(lean_refusal(
            target,
            DiagnosticCode::Infrastructure,
            format!(
                "Lake did not report the project's LEAN_PATH: {}",
                String::from_utf8_lossy(&printed_path.stderr).trim()
            ),
        ));
    }
    let lake_path: Vec<PathBuf> =
        std::env::split_paths(String::from_utf8_lossy(&printed_path.stdout).trim()).collect();
    // Lean resolves every module of a package under the first search root holding the package's
    // directory, so the work directory overlays the project's own build: the target's module is the
    // fresh compilation, every other module of its package a link to the project's prebuilt one.
    overlay(&work.0, &lake_path, &relative).map_err(|e| {
        lean_refusal(
            target,
            DiagnosticCode::Infrastructure,
            format!("cannot overlay the project's build for {module}: {e}"),
        )
    })?;
    let compiled = run_lean(
        lake_command(&[
            "lean".as_ref(),
            source_file.as_os_str(),
            "--".as_ref(),
            "-o".as_ref(),
            olean.as_os_str(),
        ]),
        profile,
        deadline,
    )
    .map_err(|f| {
        tool_failure(
            f,
            target,
            options,
            &lake_binary,
            &format!("compiling {}", source_file.display()),
        )
    })?;
    if !compiled.status.success() {
        let output = format!(
            "{}{}",
            String::from_utf8_lossy(&compiled.stdout),
            String::from_utf8_lossy(&compiled.stderr)
        );
        if lean_errors(&output).is_empty() {
            return Err(lean_refusal(
                target,
                DiagnosticCode::Infrastructure,
                format!(
                    "Lake did not compile {module} against the project's prebuilt dependencies ({}): build them first (`lake build {module}`, `lake exe cache get` for Mathlib): {}",
                    compiled.status,
                    output.trim()
                ),
            ));
        }
        return Err(lean_rejection(target, &source_file, &compiled));
    }
    if !olean.is_file() {
        return Err(lean_refusal(
            target,
            DiagnosticCode::Infrastructure,
            format!("Lake compiled {module} but wrote no {}", olean.display()),
        ));
    }
    let after = std::fs::read(project.join(&source_file)).unwrap_or_default();
    if after != source {
        return Err(lean_refusal(
            target,
            DiagnosticCode::Infrastructure,
            format!("{} changed while it was compiled", source_file.display()),
        ));
    }
    let mut search = vec![work.0.clone()];
    search.extend(lake_path);
    let mut sources = BTreeMap::new();
    sources.insert(slash_path(&source_file), source);
    for file in CONFIGURATION {
        if let Ok(bytes) = std::fs::read(project.join(file)) {
            sources.insert(file.to_string(), bytes);
        }
    }
    Ok(Environment {
        imports: vec![module.to_string()],
        search,
        sources,
        dependencies: lake.dependencies.clone(),
    })
}

/// Every `.lean` file of the project (not the lakefile, not build output, not hidden directories),
/// sorted by path so resolution is deterministic.
fn discover(project: &Path, target: &str) -> Result<Vec<Module>, Refusal> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
        let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<std::io::Result<_>>()?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            let kind = entry.file_type()?;
            if kind.is_dir() {
                if !name.starts_with('.') && name != "build" {
                    walk(root, &path, out)?;
                }
            } else if kind.is_file()
                && name.ends_with(".lean")
                && !(dir == root && name == "lakefile.lean")
            {
                out.push(
                    path.strip_prefix(root)
                        .expect("under the root")
                        .to_path_buf(),
                );
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(project, project, &mut files).map_err(|e| {
        lean_refusal(
            target,
            DiagnosticCode::Infrastructure,
            format!("cannot read the project {}: {e}", project.display()),
        )
    })?;
    if files.is_empty() {
        return Err(lean_refusal(
            target,
            DiagnosticCode::MissingDeclaration,
            format!(
                "the project {} has no Lean modules, so it cannot declare {target}",
                project.display()
            ),
        ));
    }
    let mut modules = Vec::new();
    for relative in files {
        let components: Vec<Component> = relative
            .with_extension("")
            .components()
            .map(|c| Component::Str(c.as_os_str().to_string_lossy().into_owned()))
            .collect();
        let source = std::fs::read_to_string(project.join(&relative)).map_err(|e| {
            lean_refusal(
                target,
                DiagnosticCode::Infrastructure,
                format!("cannot read {}: {e}", relative.display()),
            )
        })?;
        modules.push(Module {
            name: intake::print_lean_name(&components),
            imports: header_imports(&source),
            relative,
        });
    }
    Ok(modules)
}

/// The modules a Lean file's header imports, with their lines: `prelude`, then `import M` commands,
/// with comments between; the header ends at the first other token.
fn header_imports(source: &str) -> Vec<(String, usize)> {
    let bytes = source.as_bytes();
    let mut at = 0;
    let mut line = 1;
    let mut imports = Vec::new();
    let skip_trivia = |at: &mut usize, line: &mut usize| loop {
        while *at < bytes.len() && bytes[*at].is_ascii_whitespace() {
            if bytes[*at] == b'\n' {
                *line += 1;
            }
            *at += 1;
        }
        if source[*at..].starts_with("--") {
            while *at < bytes.len() && bytes[*at] != b'\n' {
                *at += 1;
            }
        } else if source[*at..].starts_with("/-") {
            let mut depth = 0usize;
            while *at < bytes.len() {
                if source[*at..].starts_with("/-") {
                    depth += 1;
                    *at += 2;
                } else if source[*at..].starts_with("-/") {
                    depth -= 1;
                    *at += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    if bytes[*at] == b'\n' {
                        *line += 1;
                    }
                    *at += source[*at..].chars().next().map_or(1, char::len_utf8);
                }
            }
        } else {
            return;
        }
    };
    let word = |at: usize| -> &str {
        let rest = &source[at..];
        let mut end = 0;
        let mut quoted = false;
        for (i, c) in rest.char_indices() {
            if c == '«' {
                quoted = true;
            } else if c == '»' {
                quoted = false;
            } else if !quoted && (c.is_whitespace() || c == '(' || c == ')') {
                break;
            }
            end = i + c.len_utf8();
        }
        &rest[..end]
    };
    skip_trivia(&mut at, &mut line);
    if word(at) == "prelude" {
        at += "prelude".len();
    }
    loop {
        skip_trivia(&mut at, &mut line);
        if word(at) != "import" {
            return imports;
        }
        at += "import".len();
        skip_trivia(&mut at, &mut line);
        let mut module = word(at);
        let import_line = line;
        if module == "runtime" {
            at += module.len();
            skip_trivia(&mut at, &mut line);
            module = word(at);
        }
        if module.is_empty() {
            return imports;
        }
        imports.push((module.to_string(), import_line));
        at += module.len();
    }
}

/// Resolve every import to a project module or a Lean core package, and order the project's modules
/// so each is compiled after everything it imports.
fn resolve(modules: &[Module], target: &str) -> Result<Vec<usize>, Refusal> {
    let by_name: BTreeMap<&str, usize> = modules
        .iter()
        .enumerate()
        .map(|(i, m)| (m.name.as_str(), i))
        .collect();
    for module in modules {
        for (import, line) in &module.imports {
            let root = import.split('.').next().unwrap_or_default();
            if !by_name.contains_key(import.as_str()) && !CORE.contains(&root) {
                return Err(refuse(
                    target,
                    "lean_accepted",
                    Diagnostic::new(
                        "lean_accepted",
                        DiagnosticCode::MissingModule,
                        format!(
                            "{} imports {import}, which is neither a module of the project nor part of Lean core ({})",
                            module.relative.display(),
                            CORE.join(", ")
                        ),
                    )
                    .with_line(*line),
                ));
            }
        }
    }
    // Depth-first, visiting imports in order: a module is emitted after its imports; a module met
    // again while still open closes a cycle.
    let mut order = Vec::new();
    let mut state = vec![0u8; modules.len()];
    fn visit(
        i: usize,
        modules: &[Module],
        by_name: &BTreeMap<&str, usize>,
        state: &mut [u8],
        stack: &mut Vec<usize>,
        order: &mut Vec<usize>,
    ) -> Result<(), Vec<usize>> {
        match state[i] {
            2 => return Ok(()),
            1 => {
                let from = stack.iter().position(|&s| s == i).unwrap_or(0);
                let mut cycle = stack[from..].to_vec();
                cycle.push(i);
                return Err(cycle);
            }
            _ => {}
        }
        state[i] = 1;
        stack.push(i);
        for (import, _) in &modules[i].imports {
            if let Some(&j) = by_name.get(import.as_str()) {
                visit(j, modules, by_name, state, stack, order)?;
            }
        }
        stack.pop();
        state[i] = 2;
        order.push(i);
        Ok(())
    }
    for i in 0..modules.len() {
        let mut stack = Vec::new();
        if let Err(cycle) = visit(i, modules, &by_name, &mut state, &mut stack, &mut order) {
            let names: Vec<&str> = cycle.iter().map(|&c| modules[c].name.as_str()).collect();
            return Err(lean_refusal(
                target,
                DiagnosticCode::ImportCycle,
                format!(
                    "the project's modules import each other in a cycle: {}",
                    names.join(" → ")
                ),
            ));
        }
    }
    Ok(order)
}

/// Every core import must be a module the profile's toolchain ships.
fn check_core_imports(
    modules: &[Module],
    prefix: &Path,
    profile: &Profile,
    target: &str,
) -> Result<(), Refusal> {
    let project: BTreeSet<&str> = modules.iter().map(|m| m.name.as_str()).collect();
    for module in modules {
        for (import, line) in &module.imports {
            if project.contains(import.as_str()) {
                continue;
            }
            let olean = import
                .split('.')
                .fold(prefix.join("lib/lean"), |p, part| p.join(part))
                .with_extension("olean");
            if !olean.is_file() {
                return Err(refuse(
                    target,
                    "lean_accepted",
                    Diagnostic::new(
                        "lean_accepted",
                        DiagnosticCode::MissingModule,
                        format!(
                            "{} imports {import}, which Lean core {} does not provide",
                            module.relative.display(),
                            profile.toolchain
                        ),
                    )
                    .with_line(*line),
                ));
            }
        }
    }
    Ok(())
}

/// The profile's Lean: it runs, it reports the profile's release and commit, and it names its
/// prefix (where Lean core lives). Returns the prefix and the toolchain's own identification.
fn check_toolchain(
    lean: &Path,
    profile: &Profile,
    target: &str,
    options: &Options,
    deadline: Instant,
) -> Result<(PathBuf, String), Refusal> {
    let version = run_lean(lean_command(lean, &["--version"]), profile, deadline)
        .map_err(|f| tool_failure(f, target, options, lean, "reporting its version"))?;
    let version_text = String::from_utf8_lossy(&version.stdout);
    let reported = version_text.trim();
    let release = format!("version {},", profile.lean_version);
    if !version.status.success() || !reported.contains(&release) {
        return Err(lean_refusal(
            target,
            DiagnosticCode::Infrastructure,
            format!(
                "{} is not the pinned Lean {} ({reported})",
                lean.display(),
                profile.lean_version,
            ),
        ));
    }
    let commit = reported
        .split("commit ")
        .nth(1)
        .and_then(|rest| rest.split([',', ')']).next())
        .map(str::trim)
        .unwrap_or_default();
    if commit.len() < 12 || !profile.lean_githash.starts_with(commit) {
        return Err(lean_refusal(
            target,
            DiagnosticCode::Infrastructure,
            format!(
                "{} reports Lean {} at commit {commit:?}, but Lean {} is commit {} ({reported})",
                lean.display(),
                profile.lean_version,
                profile.lean_version,
                profile.lean_githash,
            ),
        ));
    }
    let prefix = run_lean(lean_command(lean, &["--print-prefix"]), profile, deadline)
        .map_err(|f| tool_failure(f, target, options, lean, "reporting its prefix"))?;
    Ok((
        PathBuf::from(String::from_utf8_lossy(&prefix.stdout).trim()),
        format!("{}: {reported}", profile.toolchain),
    ))
}

/// The exporter registry: `$LOGOS_LEAN_EXPORTERS`, else `~/.logos/lean/exporters`.
fn exporter_registry() -> Option<PathBuf> {
    std::env::var_os("LOGOS_LEAN_EXPORTERS")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".logos/lean/exporters")))
}

/// Where the profile's exporter is: `--exporter`, else (Stage A) `$LOGOS_LEAN4EXPORT` or
/// `~/.logos/lean/lean4export`, else the registry entry of the profile's release. The first that
/// exists is the exporter; its identity is checked by [`check_exporter`], never assumed.
fn locate_exporter(flag: Option<&Path>, profile: &Profile) -> Result<PathBuf, Diagnostic> {
    if let Some(dir) = flag {
        return Ok(dir.to_path_buf());
    }
    let mut candidates = Vec::new();
    if profile.lean_version == profile::STAGE_A.lean_version {
        if let Some(dir) = std::env::var_os("LOGOS_LEAN4EXPORT") {
            candidates.push(PathBuf::from(dir));
        }
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join(".logos/lean/lean4export"));
        }
    }
    if let Some(registry) = exporter_registry() {
        candidates.push(registry.join(profile.lean_version));
    }
    if let Some(found) = candidates.iter().find(|dir| dir.is_dir()) {
        return Ok(found.clone());
    }
    let looked = candidates
        .iter()
        .map(|dir| dir.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Err(Diagnostic::new(
        "lean_accepted",
        DiagnosticCode::Infrastructure,
        format!(
            "no pinned lean4export {} for Lean {} (looked in: {looked}): pass --exporter <DIR>, set {}, or build it with scripts/lean/install-lean-exporters.py install {}",
            profile.exporter.revision,
            profile.lean_version,
            if profile.lean_version == profile::STAGE_A.lean_version {
                "LOGOS_LEAN4EXPORT or LOGOS_LEAN_EXPORTERS"
            } else {
                "LOGOS_LEAN_EXPORTERS"
            },
            profile.lean_version,
        ),
    ))
}

/// The profile's exporter, identified by the content of its sources and, for an executable profile,
/// of the built executable: the digest pinned for this platform, or — on a platform the profile pins
/// no build for — the digest the exporter's own build receipt records.
fn check_exporter(dir: &Path, profile: &Profile) -> Result<PathBuf, Diagnostic> {
    let revision = profile.exporter.revision;
    let problem =
        |message: String| Diagnostic::new("lean_accepted", DiagnosticCode::Infrastructure, message);
    for (file, expected) in profile.exporter.sources {
        let actual = std::fs::read(dir.join(file))
            .map(|bytes| hex(&logicaffeine_compile::crypto_accel::sha256(&bytes)));
        match actual {
            Ok(actual) if actual == *expected => {}
            Ok(actual) => {
                return Err(problem(format!(
                    "{} is not the pinned lean4export {revision} of Lean {}: {file} has sha256 {actual}, the pin is {expected}",
                    dir.display(),
                    profile.lean_version
                )))
            }
            Err(e) => {
                return Err(problem(format!(
                    "{} is not the pinned lean4export {revision} of Lean {}: cannot read {file}: {e}",
                    dir.display(),
                    profile.lean_version
                )))
            }
        }
    }
    match profile.exporter.form {
        ExporterForm::Interpreted => {
            if !dir.join(".lake/build/lib/Export.olean").is_file() {
                return Err(problem(format!(
                    "the pinned lean4export {revision} at {} is not built under Lean {}",
                    dir.display(),
                    profile.lean_version
                )));
            }
        }
        ExporterForm::Executable => {
            let executable = dir.join(EXECUTABLE);
            let actual = std::fs::read(&executable)
                .map(|bytes| hex(&logicaffeine_compile::crypto_accel::sha256(&bytes)))
                .map_err(|e| {
                    problem(format!(
                        "the pinned lean4export {revision} at {} is not built under Lean {}: cannot read {EXECUTABLE}: {e}",
                        dir.display(),
                        profile.lean_version
                    ))
                })?;
            let platform = host_platform();
            let expected = match profile.exporter.binary_for(&platform) {
                Some(pinned) => pinned.to_string(),
                None => receipt_binary(dir, profile).map_err(|why| {
                    problem(format!(
                        "Lean {} pins no lean4export build for {platform}, and {}'s build receipt does not identify it: {why}",
                        profile.lean_version,
                        dir.display()
                    ))
                })?,
            };
            if actual != expected {
                return Err(problem(format!(
                    "{} is not the pinned lean4export {revision} build of Lean {}: {EXECUTABLE} has sha256 {actual}, the pin for {platform} is {expected}",
                    dir.display(),
                    profile.lean_version
                )));
            }
        }
    }
    // Absolute: the exporter runs from the isolated work directory.
    dir.canonicalize()
        .map_err(|e| problem(format!("cannot resolve {}: {e}", dir.display())))
}

/// The executable digest an exporter's `receipt.json` records, when it describes this profile.
fn receipt_binary(dir: &Path, profile: &Profile) -> Result<String, String> {
    let text = std::fs::read_to_string(dir.join("receipt.json")).map_err(|e| e.to_string())?;
    let receipt: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    if receipt["schema"] != "logos-lean-exporter-receipt/1"
        || receipt["lean_version"] != profile.lean_version
        || receipt["toolchain"] != profile.toolchain
        || receipt["exporter"]["revision"] != profile.exporter.revision
    {
        return Err("the receipt describes another exporter".into());
    }
    receipt["binary"]["sha256"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "the receipt records no executable digest".into())
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Lean rejected a module: every error it reported, at Lean's own location.
fn lean_rejection(target: &str, relative: &Path, compiled: &process::Captured) -> Refusal {
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&compiled.stdout),
        String::from_utf8_lossy(&compiled.stderr)
    );
    let errors = lean_errors(&output);
    let mut result = LeanResult::new(target);
    if errors.is_empty() {
        let code = if compiled.status.code().is_some() {
            DiagnosticCode::LeanRejected
        } else {
            DiagnosticCode::Infrastructure
        };
        result.fail(
            "lean_accepted",
            Diagnostic::new(
                "lean_accepted",
                code,
                format!(
                    "Lean did not compile {} ({}): {}",
                    relative.display(),
                    compiled.status,
                    output.trim()
                ),
            ),
        );
    }
    for (line, message) in errors {
        result.fail(
            "lean_accepted",
            Diagnostic::new("lean_accepted", DiagnosticCode::LeanRejected, message).with_line(line),
        );
    }
    Box::new(result)
}

/// `(line, full message)` of every `file:line:col: error: …` message in Lean's output, with its
/// continuation lines.
fn lean_errors(output: &str) -> Vec<(usize, String)> {
    let mut errors: Vec<(usize, String)> = Vec::new();
    let mut in_error = false;
    for line in output.lines() {
        match parse_location(line) {
            Some((row, severity)) => {
                in_error = severity == "error";
                if in_error {
                    errors.push((row, line.to_string()));
                }
            }
            None if in_error => {
                if let Some((_, message)) = errors.last_mut() {
                    message.push('\n');
                    message.push_str(line);
                }
            }
            None => {}
        }
    }
    errors
}

/// `file:line:col: severity: …` → (line, severity).
fn parse_location(line: &str) -> Option<(usize, &str)> {
    let mut parts = line.splitn(4, ':');
    let _file = parts.next()?;
    let row: usize = parts.next()?.trim().parse().ok()?;
    let _col: usize = parts.next()?.trim().parse().ok()?;
    let rest = parts.next()?.trim_start();
    let severity = ["error", "warning", "information", "info"]
        .into_iter()
        .find(|s| rest.starts_with(&format!("{s}:")))?;
    Some((row, severity))
}

/// Lean's own `#print axioms` for the target, in a file of its own that imports every project module.
fn print_axioms(
    environment: &Environment,
    target: &str,
    work: &Path,
    profile: &Profile,
    options: &Options,
    deadline: Instant,
) -> Result<Vec<String>, Refusal> {
    let audit = work.join(".logos-audit");
    let mut source: String = environment
        .imports
        .iter()
        .map(|m| format!("import {m}\n"))
        .collect();
    source.push_str(&format!("#print axioms {}\n", printed(target)));
    std::fs::create_dir_all(&audit)
        .and_then(|_| std::fs::write(audit.join("Audit.lean"), source))
        .map_err(|e| {
            lean_refusal(
                target,
                DiagnosticCode::Infrastructure,
                format!("cannot write the axiom audit: {e}"),
            )
        })?;
    let lean = &options.lean;
    let mut command = Command::new(lean);
    command
        .arg("Audit.lean")
        .current_dir(&audit)
        .env("LEAN_PATH", join_paths(&environment.search));
    let ran = run_lean(command, profile, deadline)
        .map_err(|f| tool_failure(f, target, options, lean, "#print axioms"))?;
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&ran.stdout),
        String::from_utf8_lossy(&ran.stderr)
    );
    if !ran.status.success() {
        let lower = output.to_lowercase();
        let unknown = lower.contains("unknown constant") || lower.contains("unknown identifier");
        let diagnostic = if unknown {
            Diagnostic::new(
                "lean_accepted",
                DiagnosticCode::MissingDeclaration,
                format!("the project does not declare {target}: {}", output.trim()),
            )
            .with_declaration(target)
        } else {
            Diagnostic::new(
                "lean_accepted",
                DiagnosticCode::Infrastructure,
                format!("Lean's #print axioms failed: {}", output.trim()),
            )
        };
        return Err(refuse(target, "lean_accepted", diagnostic));
    }
    parse_print_axioms(&output).ok_or_else(|| {
        lean_refusal(
            target,
            DiagnosticCode::Infrastructure,
            format!(
                "Lean's #print axioms output is not in the expected form: {}",
                output.trim()
            ),
        )
    })
}

/// A Lean string literal of `text`.
fn lean_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A Lean term building the name `components`, robust to any component (numeric, private, quoted).
fn lean_name_term(components: &[Component]) -> String {
    components
        .iter()
        .fold("Lean.Name.anonymous".to_string(), |pre, c| match c {
            Component::Str(s) => format!("(Lean.Name.str {pre} {})", lean_string(s)),
            Component::Num(n) => format!("(Lean.Name.num {pre} {n})"),
        })
}

/// Lean's own `#print axioms` for the target's statement artifact: Lean builds the artifact exactly
/// as LOGOS derives it — the definition `T.logos_statement : Prop` whose value is the target's type
/// (a theorem) or value (a `Prop` definition), every `answer := true` hole abstracted as a parameter
/// in first-occurrence order — adds it through its own kernel (`addDecl`), and reports what it rests
/// on. A target that states no proposition, or a name already taken, is Lean's refusal.
fn statement_axioms(
    environment: &Environment,
    target: &str,
    work: &Path,
    profile: &Profile,
    options: &Options,
    deadline: Instant,
) -> Result<Vec<String>, Refusal> {
    let components = intake::parse_lean_name(target).map_err(|verdict| {
        lean_refusal(target, DiagnosticCode::Usage, verdict.reason().message())
    })?;
    let mut artifact = components.clone();
    artifact.push(Component::Str(
        logicaffeine_kernel::interface::lean::statement::ARTIFACT_COMPONENT.into(),
    ));
    let audit = work.join(".logos-statement");
    let mut source: String = environment
        .imports
        .iter()
        .map(|m| format!("import {m}\n"))
        .collect();
    source.push_str(&format!(
        r#"
open Lean in
partial def logosAnswerHoles (e : Expr) (acc : Array Expr) : Array Expr :=
  match e with
  | .mdata d inner =>
    if d.getBool `answer false then (if acc.any (·.equal e) then acc else acc.push e)
    else logosAnswerHoles inner acc
  | .app f a => logosAnswerHoles a (logosAnswerHoles f acc)
  | .lam _ t b _ => logosAnswerHoles b (logosAnswerHoles t acc)
  | .forallE _ t b _ => logosAnswerHoles b (logosAnswerHoles t acc)
  | .letE _ t v b _ => logosAnswerHoles b (logosAnswerHoles v (logosAnswerHoles t acc))
  | .proj _ _ s => logosAnswerHoles s acc
  | _ => acc

open Lean in
def logosAnswerType (hole : Expr) : Except String Expr :=
  let inner := hole.mdataExpr!
  let args := inner.getAppArgs
  if !inner.getAppFn.isConstOf ``sorryAx then .error "an answer hole that is not answer(sorry)"
  else if args.size == 2 then .ok args[0]!
  else if args.size == 3 then .ok args[0]!.bindingBody!
  else .error "sorryAx applied to an unexpected number of arguments"

open Lean Elab Command in
run_cmd liftCoreM do
  let target := {target_name}
  let ci ← getConstInfo target
  let statement ← match ci with
    | .thmInfo info => pure info.type
    | .defnInfo info =>
      if info.type == .sort .zero && info.safety == .safe then pure info.value
      else throwError "the target states no proposition: a definition states one only when it is safe and its type is Prop"
    | _ => throwError "the target states no proposition"
  let holes := logosAnswerHoles statement #[]
  let names : Array Name := if holes.size == 1 then #[`answer]
    else (Array.range holes.size).map (fun i => Name.mkSimple s!"answer_{{i+1}}")
  let fvars : Array Expr := (Array.range holes.size).map (fun i => .fvar ⟨Name.mkNum `logos.answer i⟩)
  let replaced := statement.replace (fun e =>
    match holes.findIdx? (·.equal e) with
    | some i => some fvars[i]!
    | none => none)
  let mut value := replaced.abstract fvars
  let mut type : Expr := .sort .zero
  for i in (List.range holes.size).reverse do
    let α ← match logosAnswerType holes[i]! with
      | .ok α => pure α
      | .error why => throwError why
    value := .lam names[i]! α value .default
    type := .forallE names[i]! α type .default
  addDecl (.defnDecl {{
    name := {artifact_name}, levelParams := ci.levelParams, type := type, value := value,
    hints := .abbrev, safety := .safe }})

#print axioms {artifact_printed}
"#,
        target_name = lean_name_term(&components),
        artifact_name = lean_name_term(&artifact),
        artifact_printed = intake::print_lean_name(&artifact),
    ));
    std::fs::create_dir_all(&audit)
        .and_then(|_| std::fs::write(audit.join("Statement.lean"), source))
        .map_err(|e| {
            lean_refusal(
                target,
                DiagnosticCode::Infrastructure,
                format!("cannot write the statement audit: {e}"),
            )
        })?;
    let lean = &options.lean;
    let mut command = Command::new(lean);
    command
        .arg("Statement.lean")
        .current_dir(&audit)
        .env("LEAN_PATH", join_paths(&environment.search));
    let ran = run_lean(command, profile, deadline)
        .map_err(|f| tool_failure(f, target, options, lean, "the statement audit"))?;
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&ran.stdout),
        String::from_utf8_lossy(&ran.stderr)
    );
    if !ran.status.success() {
        let lower = output.to_lowercase();
        let unknown = lower.contains("unknown constant") || lower.contains("unknown identifier");
        let diagnostic = if output.contains("the target states no proposition") {
            Diagnostic::new(
                "lean_accepted",
                DiagnosticCode::UnsupportedFeature,
                format!("{target} states no proposition: {}", output.trim()),
            )
            .with_declaration(target)
        } else if unknown {
            Diagnostic::new(
                "lean_accepted",
                DiagnosticCode::MissingDeclaration,
                format!("the project does not declare {target}: {}", output.trim()),
            )
            .with_declaration(target)
        } else {
            Diagnostic::new(
                "lean_accepted",
                DiagnosticCode::Infrastructure,
                format!("Lean's statement audit failed: {}", output.trim()),
            )
        };
        return Err(refuse(target, "lean_accepted", diagnostic));
    }
    parse_print_axioms(&output).ok_or_else(|| {
        lean_refusal(
            target,
            DiagnosticCode::Infrastructure,
            format!(
                "Lean's #print axioms output for the statement is not in the expected form: {}",
                output.trim()
            ),
        )
    })
}

/// `'X' does not depend on any axioms` → `[]`; `'X' depends on axioms: [a, b]` → `[a, b]`.
fn parse_print_axioms(output: &str) -> Option<Vec<String>> {
    if output.contains("does not depend on any axioms") {
        return Some(Vec::new());
    }
    let start = output.find("depends on axioms: [")? + "depends on axioms: [".len();
    let rest = &output[start..];
    let mut names = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in rest.chars() {
        match c {
            '«' => {
                quoted = true;
                current.push(c);
            }
            '»' => {
                quoted = false;
                current.push(c);
            }
            ',' | ']' if !quoted => {
                let name = current.trim().to_string();
                if !name.is_empty() {
                    names.push(name);
                }
                current.clear();
                if c == ']' {
                    return Some(names);
                }
            }
            _ => current.push(c),
        }
    }
    None
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_cli/commands/lean/project/tests.rs"]
mod tests;
