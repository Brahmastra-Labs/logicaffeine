//! Project mode: compile a Lean project in isolation with the pinned toolchain, establish the
//! target's assumptions with Lean's own `#print axioms`, and export the target with the pinned
//! lean4export.
//!
//! Nothing here trusts an exit code alone. Lean compiles a `sorry` with exit code 0 and a warning,
//! and the pinned exporter PANICS on an unknown constant and still exits 0; so the target is
//! resolved and its axioms read from `#print axioms` before the exporter runs, and the exporter's
//! output is held to its own contract (the intake) afterwards.
//!
//! Deterministic resolution: the project's modules are its `.lean` files; an import is a project
//! module or a Lean core module (`Init`, `Std`, `Lean`), resolved before Lean runs and compiled in
//! dependency order. Stage A admits no Lake dependency.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use logicaffeine_kernel::interface::lean::intake::{
    self, Component, EXPORTER_FLAGS, EXPORTER_REVISION, EXPORTER_SOURCES, TOOLCHAIN,
};
use logicaffeine_kernel::interface::lean::pipeline::{Diagnostic, LeanEvidence, LeanResult};
use logicaffeine_kernel::interface::lean::DiagnosticCode;

use super::process::{self, Failure};

pub(crate) struct Options {
    pub lean: PathBuf,
    pub exporter: Option<PathBuf>,
    pub timeout_secs: u64,
    pub max_export_bytes: u64,
}

pub(crate) struct Exported {
    pub export: String,
    pub evidence: LeanEvidence,
}

type Refusal = Box<LeanResult>;

/// The Lean-core packages a Stage-A project may import.
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
    check_toolchain_pin(project, target)?;
    check_no_lake_dependencies(project, target)?;
    let modules = discover(project, target)?;
    let order = resolve(&modules, target)?;

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
    let toolchain = check_toolchain(lean, target, options, deadline);
    let exporter = check_exporter(options.exporter.as_deref());
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
    check_core_imports(&modules, &prefix, target)?;

    let work = WorkDir::new().map_err(|e| {
        lean_refusal(
            target,
            DiagnosticCode::Infrastructure,
            format!("cannot create an isolated work directory: {e}"),
        )
    })?;
    for module in &modules {
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

    for &index in &order {
        let module = &modules[index];
        let olean = module.relative.with_extension("olean");
        let mut command = Command::new(lean);
        command
            .arg("-o")
            .arg(&olean)
            .arg(&module.relative)
            .current_dir(&work.0)
            .env("LEAN_PATH", &work.0);
        let compiled = run_lean(command, deadline).map_err(|f| {
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

    let axioms = print_axioms(&modules, target, &work.0, options, deadline)?;

    let mut command = Command::new(lean);
    command
        .arg("--run")
        .arg(exporter.join("Main.lean"))
        .arg("--")
        .args(EXPORTER_FLAGS)
        .args(modules.iter().map(|m| m.name.as_str()))
        .arg("--")
        .arg(printed(target))
        .current_dir(&work.0)
        .env(
            "LEAN_PATH",
            join_paths(&[exporter.join(".lake/build/lib"), work.0.clone()]),
        );
    command
        .env("ELAN_TOOLCHAIN", TOOLCHAIN)
        .env_remove("LEAN_SRC_PATH");
    let exported =
        process::run(command, deadline, options.max_export_bytes).map_err(|f| match f {
            Failure::Deadline => {
                deadline_refusal(target, "translated", options, "the pinned exporter")
            }
            Failure::Spawn(e) => refuse(
                target,
                "translated",
                Diagnostic::new(
                    "translated",
                    DiagnosticCode::Infrastructure,
                    format!("cannot run the pinned exporter: {e}"),
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
                    "the pinned exporter failed ({}): {}",
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
                "the pinned exporter wrote output that is not UTF-8",
            ),
        )
    })?;
    let mut sources = std::collections::BTreeMap::new();
    for module in &modules {
        let key = module
            .relative
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        let bytes = std::fs::read(work.0.join(&module.relative)).map_err(|e| {
            lean_refusal(
                target,
                DiagnosticCode::Infrastructure,
                format!("cannot read {}: {e}", module.relative.display()),
            )
        })?;
        sources.insert(key, bytes);
    }
    let exporter = format!("lean4export {EXPORTER_REVISION}");
    Ok(Exported {
        export,
        evidence: LeanEvidence::Accepted {
            axioms,
            sources,
            toolchain: toolchain_identity,
            exporter,
        },
    })
}

/// Run the pinned Lean with the pinned toolchain selected and no inherited search path.
fn run_lean(mut command: Command, deadline: Instant) -> Result<process::Captured, Failure> {
    command
        .env("ELAN_TOOLCHAIN", TOOLCHAIN)
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

/// A project's `lean-toolchain`, when present, must pin the Stage-A toolchain.
fn check_toolchain_pin(project: &Path, target: &str) -> Result<(), Refusal> {
    let Ok(pin) = std::fs::read_to_string(project.join("lean-toolchain")) else {
        return Ok(());
    };
    let pin = pin.trim();
    let normalized = pin.replace(":v", ":");
    if normalized != TOOLCHAIN.replace(":v", ":") {
        return Err(lean_refusal(
            target,
            DiagnosticCode::UnsupportedToolchain,
            format!("the project pins {pin}; Stage A supports {TOOLCHAIN} only"),
        ));
    }
    Ok(())
}

/// Stage A admits Lean core only: a lakefile that requires a package is outside it.
fn check_no_lake_dependencies(project: &Path, target: &str) -> Result<(), Refusal> {
    for file in ["lakefile.lean", "lakefile.toml"] {
        if let Ok(text) = std::fs::read_to_string(project.join(file)) {
            let requires = text.lines().any(|l| {
                let l = l.trim_start();
                l.starts_with("require ") || l.starts_with("[[require]]")
            });
            if requires {
                return Err(lean_refusal(
                    target,
                    DiagnosticCode::UnsupportedFeature,
                    format!("{file} requires a Lake package; Stage A admits no dependency beyond Lean core"),
                ));
            }
        }
    }
    if let Ok(text) = std::fs::read_to_string(project.join("lake-manifest.json")) {
        let packages = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v["packages"].as_array().map(Vec::len))
            .unwrap_or(0);
        if packages > 0 {
            return Err(lean_refusal(
                target,
                DiagnosticCode::UnsupportedFeature,
                "lake-manifest.json pins Lake packages; Stage A admits no dependency beyond Lean core",
            ));
        }
    }
    Ok(())
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

/// Every core import must be a module the pinned toolchain ships.
fn check_core_imports(modules: &[Module], prefix: &Path, target: &str) -> Result<(), Refusal> {
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
                            "{} imports {import}, which Lean core {TOOLCHAIN} does not provide",
                            module.relative.display()
                        ),
                    )
                    .with_line(*line),
                ));
            }
        }
    }
    Ok(())
}

/// The pinned Lean: it runs, it is 4.14.0, and it names its prefix (where Lean core lives). Returns
/// the prefix and the toolchain's own identification.
fn check_toolchain(
    lean: &Path,
    target: &str,
    options: &Options,
    deadline: Instant,
) -> Result<(PathBuf, String), Refusal> {
    let version = run_lean(lean_command(lean, &["--version"]), deadline)
        .map_err(|f| tool_failure(f, target, options, lean, "reporting its version"))?;
    let version_text = String::from_utf8_lossy(&version.stdout);
    if !version.status.success() || !version_text.contains("version 4.14.0") {
        return Err(lean_refusal(
            target,
            DiagnosticCode::Infrastructure,
            format!(
                "{} is not the pinned Lean 4.14.0 ({})",
                lean.display(),
                version_text.trim()
            ),
        ));
    }
    let prefix = run_lean(lean_command(lean, &["--print-prefix"]), deadline)
        .map_err(|f| tool_failure(f, target, options, lean, "reporting its prefix"))?;
    Ok((
        PathBuf::from(String::from_utf8_lossy(&prefix.stdout).trim()),
        format!("{TOOLCHAIN}: {}", version_text.trim()),
    ))
}

/// The pinned exporter, identified by the content of its sources.
fn check_exporter(dir: Option<&Path>) -> Result<PathBuf, Diagnostic> {
    let problem =
        |message: String| Diagnostic::new("lean_accepted", DiagnosticCode::Infrastructure, message);
    let Some(dir) = dir else {
        return Err(problem(format!(
            "no pinned lean4export {EXPORTER_REVISION}: pass --exporter <DIR> or set LOGOS_LEAN4EXPORT"
        )));
    };
    for (file, expected) in EXPORTER_SOURCES {
        let actual = std::fs::read(dir.join(file))
            .map(|bytes| hex(&logicaffeine_compile::crypto_accel::sha256(&bytes)));
        match actual {
            Ok(actual) if actual == expected => {}
            Ok(actual) => {
                return Err(problem(format!(
                    "{} is not the pinned lean4export {EXPORTER_REVISION}: {file} has sha256 {actual}, the pin is {expected}",
                    dir.display()
                )))
            }
            Err(e) => {
                return Err(problem(format!(
                    "{} is not the pinned lean4export {EXPORTER_REVISION}: cannot read {file}: {e}",
                    dir.display()
                )))
            }
        }
    }
    if !dir.join(".lake/build/lib/Export.olean").is_file() {
        return Err(problem(format!(
            "the pinned lean4export {EXPORTER_REVISION} at {} is not built under Lean 4.14.0",
            dir.display()
        )));
    }
    // Absolute: the exporter runs from the isolated work directory.
    dir.canonicalize()
        .map_err(|e| problem(format!("cannot resolve {}: {e}", dir.display())))
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
    modules: &[Module],
    target: &str,
    work: &Path,
    options: &Options,
    deadline: Instant,
) -> Result<Vec<String>, Refusal> {
    let audit = work.join(".logos-audit");
    let mut source: String = modules
        .iter()
        .map(|m| format!("import {}\n", m.name))
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
        .env("LEAN_PATH", work);
    let ran = run_lean(command, deadline)
        .map_err(|f| tool_failure(f, target, options, lean, "#print axioms"))?;
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&ran.stdout),
        String::from_utf8_lossy(&ran.stderr)
    );
    if !ran.status.success() {
        let unknown = output.contains("unknown constant") || output.contains("unknown identifier");
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
