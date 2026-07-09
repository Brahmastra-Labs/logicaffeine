//! `largo run` — build-and-execute, interpret, or compile-to-wasm-and-run.

use std::fs;

use crate::commands::build::friendly_build_error;
use crate::commands::emit::build_wasm_module;
use crate::commands::require_project_root;
use crate::project::build::{self, BuildConfig};
use crate::project::manifest::Manifest;

/// Handle `largo run` (default path): build with cargo, then execute.
pub(crate) fn cmd_run(release: bool, args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let project_root = require_project_root()?;

    let config = BuildConfig {
        project_dir: project_root,
        release,
        lib_mode: false,
        target: None,
    };

    let result = build::build(config).map_err(friendly_build_error)?;
    let exit_code = build::run(&result, args)?;

    if exit_code != 0 {
        std::process::exit(exit_code);
    }

    Ok(())
}

/// Handle `largo run --interpret`: tree-walk the program without any Rust build.
pub(crate) fn cmd_run_interpret(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let project_root = require_project_root()?;

    let manifest = Manifest::load(&project_root)?;
    let entry_path = crate::commands::resolve_entry_path(&project_root, &manifest)?;
    let source = fs::read_to_string(&entry_path)?;

    // Build the argv the program's `args()` sees: index 0 is the program name
    // (the compiled binary's `env::args()[0]`), then the user arguments — so
    // `item 2 of args()` is the first user argument on the interpreter exactly
    // as on the native binary.
    let mut argv = Vec::with_capacity(args.len() + 1);
    argv.push(manifest.package.name.clone());
    argv.extend(args.iter().cloned());

    // Compiled-native tier (HOTSWAP §Axis-3): if the program annotates functions
    // `is exported for native`, load them as rustc -O3 machine code (cached, so a
    // pre-built `largo build --native-functions` bundle is a cache hit) and queue them
    // so the VM dispatches those functions to compiled native from their first call.
    // Absent / unbuildable ⇒ nothing queued ⇒ runs on VM+JIT, no gap at the seam.
    #[cfg(not(target_arch = "wasm32"))]
    {
        let names = logicaffeine_compile::compile::native_export_function_names(&source);
        if !names.is_empty() {
            let cache_dir = project_root.join(".logos-native");
            let natives = logicaffeine_compile::compile::aot_load_bundle(&source, &cache_dir);
            if !natives.is_empty() {
                logicaffeine_compile::ui_bridge::set_pending_aot_natives(natives);
            }
        }
    }

    let result = futures::executor::block_on(
        logicaffeine_compile::interpret_for_ui_with_args(&source, &argv),
    );

    for line in &result.lines {
        println!("{}", line);
    }

    if let Some(err) = result.error {
        eprintln!("{}", err);
        std::process::exit(1);
    }

    Ok(())
}

/// `largo run --emit wasm [args…]` — compile to wasm and run it in ONE step through the emitted host
/// shim (node), passing `args` to the program's `args()`. Compile-and-run with no Rust toolchain.
pub(crate) fn cmd_run_wasm(args: &[String], linked: bool) -> Result<(), Box<dyn std::error::Error>> {
    let project_root = require_project_root()?;
    let (mjs, _) = build_wasm_module(&project_root, linked)?;
    let status = std::process::Command::new("node")
        .arg(&mjs)
        .args(args)
        .status()
        .map_err(|e| format!("could not launch node to run the wasm (is node installed?): {e}"))?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}
