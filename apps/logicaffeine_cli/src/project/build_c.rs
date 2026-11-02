//! The C build: `largo build [--release] --emit c|c-bytecode`.
//!
//! The Rust build with another backend. The same two profiles, the artifact published at
//! `target/<mode>/<name>`, and the generated source kept in the build directory beside it
//! (`target/<mode>/build-c/main.c`, where the Rust build keeps `build/src/main.rs`). The C compiler is
//! `$CC`, default `cc`.
//!
//! | profile | flags |
//! |---|---|
//! | debug | [`C_DEBUG_FLAGS`] |
//! | release | [`C_RELEASE_FLAGS`] — the flags the benchmark suite builds its hand-written C with |
//!
//! A release build strips its symbol table, as the Rust release profile does. The same override the Rust
//! build honours keeps it: `CARGO_PROFILE_RELEASE_STRIP=false` (or `none`).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::build::{BuildConfig, BuildError, BuildResult};
use super::manifest::Manifest;

/// The optimization flags of a debug C build.
pub const C_DEBUG_FLAGS: &[&str] = &["-O0", "-g"];

/// The optimization flags of a release C build: full optimization, the host CPU, link-time optimization, and no
/// floating-point contraction — `a*b + c` stays two rounded operations, as it is on the Rust tier and the VM, so every
/// tier computes the same bits.
pub const C_RELEASE_FLAGS: &[&str] = &["-O3", "-march=native", "-flto", "-ffp-contract=off"];

/// Which LOGOS → C lowering a C build runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CLowering {
    /// The optimized AST, lowered by [`logicaffeine_compile::compile::compile_to_c`] (`--emit c`).
    Ast,
    /// The optimized bytecode, lowered by
    /// [`logicaffeine_compile::compile::compile_to_c_via_bytecode`] (`--emit c-bytecode`).
    Bytecode,
}

impl CLowering {
    /// The `--emit` value that selects this lowering.
    pub fn emit_name(self) -> &'static str {
        match self {
            CLowering::Ast => "c",
            CLowering::Bytecode => "c-bytecode",
        }
    }
}

/// The C compiler a build runs: `$CC`, else `cc`.
pub fn c_compiler() -> String {
    std::env::var("CC")
        .ok()
        .filter(|cc| !cc.trim().is_empty())
        .unwrap_or_else(|| "cc".to_string())
}

/// Whether a release build strips its symbol table: yes, unless `CARGO_PROFILE_RELEASE_STRIP` says
/// `false` or `none` — the knob the Rust release profile answers to.
fn release_strips() -> bool {
    !matches!(
        std::env::var("CARGO_PROFILE_RELEASE_STRIP")
            .ok()
            .as_deref()
            .map(str::trim),
        Some("false") | Some("none")
    )
}

/// The complete compiler argument list for `source` → `output` under `release`, linking `links` after the source:
/// what the generated C asks for beyond the C library (the exact extension's value runtime,
/// [`logicaffeine_compile::codegen_c::value_runtime::link_args`]).
pub fn c_compiler_args(release: bool, source: &Path, output: &Path, links: &[String]) -> Vec<String> {
    let mut args: Vec<String> = (if release {
        C_RELEASE_FLAGS
    } else {
        C_DEBUG_FLAGS
    })
    .iter()
    .map(|f| f.to_string())
    .collect();
    if release && release_strips() {
        args.push("-s".to_string());
    }
    args.push("-o".to_string());
    args.push(output.display().to_string());
    args.push(source.display().to_string());
    args.extend(links.iter().cloned());
    args.push("-lm".to_string());
    args
}

pub(crate) fn build_c_with_entry(
    config: &BuildConfig,
    manifest: &Manifest,
    entry_path: &Path,
    lowering: CLowering,
) -> Result<BuildResult, BuildError> {
    if config.lib_mode {
        return Err(BuildError::Unsupported(format!(
            "`--emit {}` builds an executable; `--lib` is a Rust build (drop `--emit` to build the library)",
            lowering.emit_name()
        )));
    }
    if config.target.is_some() {
        return Err(BuildError::Unsupported(format!(
            "`--emit {}` builds for this machine; `--target` cross-compiles the Rust build (drop `--emit` to cross-compile)",
            lowering.emit_name()
        )));
    }

    let started = std::time::Instant::now();
    crate::ui::phase(
        "Compiling",
        format!(
            "{} v{} (LOGOS → C)",
            manifest.package.name, manifest.package.version
        ),
    );

    let build_dir =
        config
            .project_dir
            .join("target")
            .join(if config.release { "release" } else { "debug" });
    let c_dir = build_dir.join("build-c");
    if c_dir.exists() {
        fs::remove_dir_all(&c_dir).map_err(|e| BuildError::Io(e.to_string()))?;
    }
    fs::create_dir_all(&c_dir).map_err(|e| BuildError::Io(e.to_string()))?;

    let dep_roots = manifest
        .resolve_dependencies(&config.project_dir)
        .map_err(|e| BuildError::Io(e.to_string()))?
        .into_iter()
        .map(|d| logicaffeine_compile::loader::DepRoot {
            name: d.name,
            root: d.root,
        })
        .collect::<Vec<_>>();
    let closure = crate::closure::assemble_closure_with_deps(entry_path, dep_roots)
        .map_err(|e| BuildError::Io(format!("could not assemble project: {e}")))?;
    let emitted = match lowering {
        CLowering::Ast => logicaffeine_compile::compile::compile_to_c(&closure.source),
        CLowering::Bytecode => {
            logicaffeine_compile::compile::compile_to_c_via_bytecode(&closure.source)
        }
    };
    let c_code = emitted.map_err(|pe| {
        let at = crate::diagnostics::Placed::locate(
            pe.span,
            &closure.source,
            &closure.source_map,
            entry_path,
        );
        BuildError::Diagnostic(crate::diagnostics::render_parse_error(&pe, &at))
    })?;

    let source = c_dir.join("main.c");
    fs::write(&source, &c_code).map_err(|e| BuildError::Io(e.to_string()))?;
    let built = c_dir.join(&manifest.package.name);

    let cc = c_compiler();
    if logicaffeine_compile::codegen_c::value_runtime::links(&c_code) {
        crate::ui::phase("Building", "the exact extension's value runtime (Rust)".to_string());
    }
    let links = logicaffeine_compile::codegen_c::value_runtime::link_args(&c_code).map_err(|why| {
        BuildError::CToolchain(format!("the exact extension's value runtime did not build: {why}"))
    })?;
    let args = c_compiler_args(config.release, &source, &built, &links);
    crate::ui::phase("Building", format!("generated C ({cc})"));
    if crate::ui::verbosity() > 0 {
        crate::ui::phase("Running", format!("`{cc} {}`", args.join(" ")));
    }
    let status = Command::new(&cc).args(&args).status().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            BuildError::CToolchain(format!(
                "the C compiler `{cc}` was not found — `largo build --emit {}` needs a C compiler (set CC to choose one)",
                lowering.emit_name()
            ))
        } else {
            BuildError::Io(format!("could not run the C compiler `{cc}`: {e}"))
        }
    })?;
    if !status.success() {
        return Err(BuildError::CCompiler(format!(
            "the C compiler `{cc}` failed ({status}) on the generated C at {}; its diagnostics are above",
            source.display()
        )));
    }

    crate::ui::phase(
        "Finished",
        format!(
            "{} profile in {:.1}s",
            if config.release { "release" } else { "dev" },
            started.elapsed().as_secs_f64()
        ),
    );
    let binary_path: PathBuf = build_dir.join(&manifest.package.name);
    fs::copy(&built, &binary_path).map_err(|e| {
        BuildError::Io(format!(
            "the build succeeded but its artifact could not be published to {}: {e}\n\
             The freshly built file is at {}. Anything at the stable path is from an EARLIER \
             build — do not ship it.",
            binary_path.display(),
            built.display()
        ))
    })?;
    crate::ui::phase("Artifact", binary_path.display().to_string());

    Ok(BuildResult {
        target_dir: build_dir,
        binary_path,
    })
}
