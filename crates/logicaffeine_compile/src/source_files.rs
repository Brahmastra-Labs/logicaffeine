//! What a LOGOS source file is called.
//!
//! Every tool that finds LOGOS source on disk asks this module — the module loader, largo's page
//! discovery, its watchers, its formatter and its packager, and the language server — so no tool
//! can recognise a file that another tool ignores. `source_extension_lock` holds that line.

use std::path::Path;

/// The extensions a LOGOS program is written in, canonical first. `.logos` is the language's own
/// name for its files and the one the Studio writes; `.lg` is the short form every earlier
/// project uses, and it keeps working.
pub const SOURCE_EXTENSIONS: [&str; 2] = ["logos", "lg"];

/// The extensions a module may carry: a program's, then literate Markdown.
pub const MODULE_EXTENSIONS: [&str; 3] = ["logos", "lg", "md"];

/// Is `path` a file a LOGOS program is written in (`.logos`, `.lg`)?
pub fn is_source_file(path: &Path) -> bool {
    has_one_of(path, &SOURCE_EXTENSIONS)
}

/// Is `path` a file a module may be — a program's source, or literate Markdown?
pub fn is_module_file(path: &Path) -> bool {
    has_one_of(path, &MODULE_EXTENSIONS)
}

fn has_one_of(path: &Path, extensions: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| extensions.contains(&e))
}
