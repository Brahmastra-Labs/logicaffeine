//! `largo build` — compile the project (Rust path, wasm emit, native bundle).

use std::fs;

use crate::commands::emit::emit_wasm_module;
use crate::commands::require_project_root;
use crate::commands::verify::run_verification;
use crate::project::build::{self, BuildConfig, BuildError, CargoFailureKind};
use crate::project::manifest::Manifest;
use crate::ui::{self, CliError};

/// Attach the actionable `help:` hint that matches a build failure's class.
///
/// The raw cargo output already streamed to the terminal; this shapes the
/// closing `error:`/`help:` lines.
pub(crate) fn friendly_build_error(e: BuildError) -> Box<dyn std::error::Error> {
    match e {
        BuildError::Cargo(ref failure) => {
            let hint = match failure.kind {
                CargoFailureKind::DependencyResolution => {
                    "check the crate name and version in your `## Requires` block"
                }
                CargoFailureKind::GeneratedCode => {
                    "please report this at https://github.com/Brahmastra-Labs/logicaffeine/issues"
                }
            };
            CliError::with_hint(e.to_string(), hint).into()
        }
        BuildError::Toolchain(_) => CliError::with_hint(
            e.to_string(),
            "install Rust from https://rustup.rs — or use `largo run --interpret` / `largo build --emit wasm`, which need no Rust toolchain",
        )
        .into(),
        other => Box::new(other),
    }
}

/// Handle `largo build` with all of its flags.
#[allow(clippy::too_many_arguments)]
pub(crate) fn cmd_build(
    release: bool,
    verify: bool,
    license: Option<String>,
    lib: bool,
    target: Option<String>,
    native_functions: bool,
    emit: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let project_root = require_project_root()?;

    // Run verification if requested
    if verify {
        run_verification(&project_root, license.as_deref())?;
    }

    // `--emit wasm`: compile the entry DIRECTLY to a `.wasm` via the built-in backend — no rustc,
    // cargo, or wasm-bindgen. Bypasses the whole Cargo path; the module is self-contained (its host
    // `print_*`/`args`/… imports are supplied by any wasm runtime or a ~10-line browser shim).
    if let Some(kind) = emit.as_deref() {
        match kind {
            "wasm" => return emit_wasm_module(&project_root, false),
            "wasm-linked" => return emit_wasm_module(&project_root, true),
            _ => return Err(format!("unknown --emit target '{kind}' (expected 'wasm' or 'wasm-linked')").into()),
        }
    }

    let config = BuildConfig {
        project_dir: project_root.clone(),
        release,
        lib_mode: lib,
        target,
    };

    let result = build::build(config).map_err(friendly_build_error)?;

    let mode = if release { "release" } else { "debug" };
    ui::info(format!("Built {} [{}]", result.binary_path.display(), mode));

    if native_functions {
        build_native_function_bundle(&project_root)?;
    }

    Ok(())
}

/// Pre-build the AOT-native tier bundle (HOTSWAP §Axis-3): every `is exported for
/// native` function compiled to a cached cdylib under `.logos-native/`. Functions
/// outside the sound scalar subset are skipped — they keep running on VM+JIT.
pub(crate) fn build_native_function_bundle(
    project_root: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let manifest = Manifest::load(project_root)?;
    let entry_path = project_root.join(&manifest.package.entry);
    let source = fs::read_to_string(&entry_path)?;

    let bundle_dir = project_root.join(".logos-native");
    let built = logicaffeine_compile::compile::build_native_bundle(&source, &bundle_dir)
        .map_err(|e| format!("native bundle build failed: {e:?}"))?;

    if built.is_empty() {
        println!("No `is exported for native` functions to bundle.");
    } else {
        println!("Bundled {} native function(s) into {}:", built.len(), bundle_dir.display());
        for (name, so) in &built {
            println!("  {name} -> {}", so.display());
        }
    }
    Ok(())
}
