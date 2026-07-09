//! `largo clean` — remove build artifacts.

use std::fs;

use crate::commands::require_project_root;
use crate::ui;

/// Handle `largo clean`: remove `target/`, and with `--all` also the
/// `.logos-native/` compiled-function bundle cache. Removing nothing is
/// still a success (idempotent).
pub(crate) fn cmd_clean(all: bool) -> Result<(), Box<dyn std::error::Error>> {
    let project_root = require_project_root()?;

    let mut removed = Vec::new();

    let target = project_root.join("target");
    if target.exists() {
        fs::remove_dir_all(&target)
            .map_err(|e| format!("could not remove {}: {e}", target.display()))?;
        removed.push(target);
    }

    if all {
        let native = project_root.join(".logos-native");
        if native.exists() {
            fs::remove_dir_all(&native)
                .map_err(|e| format!("could not remove {}: {e}", native.display()))?;
            removed.push(native);
        }
    }

    if removed.is_empty() {
        ui::info("Nothing to clean");
    } else {
        for path in &removed {
            ui::info(format!("Removed {}", path.display()));
        }
    }
    Ok(())
}
