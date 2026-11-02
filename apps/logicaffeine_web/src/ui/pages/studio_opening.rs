//! What the Studio shows before anything has loaded.
//!
//! The Studio has no mode switch: the open file is the mode. Its first frame — the page the site
//! prerenders, and the first one the app paints — must already know the file, or it paints one
//! mode and flips to another once the file system answers. Every shipped example is compiled into
//! the site, so the route alone decides what the Studio opens on, the same on the server and in the
//! browser ([`StudioOpening::for_route`]). Only a reader's own file waits for the file system, and
//! [`StudioOpening::arrival`] is the one rule for what the file system's answer may change.

use crate::ui::examples::{ExampleSpec, ALL_CODE_EXAMPLES};
use crate::ui::pages::studio_files::{example_at, examples_tree};
use crate::ui::state::StudioMode;

/// The folder whose first file, in the file browser's order, a bare `/studio` opens.
const FIRST_FOLDER: &str = "/examples/code";

const _: () = assert!(!ALL_CODE_EXAMPLES.is_empty(), "a bare /studio opens a Code example, so one must ship");

/// The file the Studio opens with, the mode it opens in, and its text when the site carries it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StudioOpening {
    /// The file's VFS path, with its leading slash.
    pub path: String,
    /// The mode the file opens in.
    pub mode: StudioMode,
    /// The file's text when the site carries it, as it does every shipped example. A reader's own
    /// file has none until the file system reads it.
    pub text: Option<&'static str>,
}

/// What the file system's answer for the opening's file changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Arrival {
    /// Nothing: the editor already shows the file, or the reader has typed since the Studio opened.
    Keep,
    /// Open the file with the text the file system holds.
    Open(String),
    /// The file system has no such file: open this one instead.
    Fallback(StudioOpening),
}

impl StudioOpening {
    /// What the Studio opens on for the route's `file`, a VFS path with or without its leading
    /// slash: a shipped example in its own mode with its text; a reader's file in the mode its path
    /// names, its text left to the file system; and with no file, the first example.
    pub fn for_route(file: Option<&str>) -> Self {
        match file.filter(|file| !file.is_empty()) {
            None => Self::first_example(),
            Some(file) if file.starts_with('/') => Self::at(file),
            Some(file) => Self::at(&format!("/{file}")),
        }
    }

    /// A bare `/studio`: the first Code example, in the order the file browser shows the examples.
    pub fn first_example() -> Self {
        let first = examples_tree().first_file_under(FIRST_FOLDER).and_then(example_at);
        Self::of(first.unwrap_or(&ALL_CODE_EXAMPLES[0]))
    }

    fn at(path: &str) -> Self {
        match example_at(path) {
            Some(spec) => Self::of(spec),
            // A file that names no mode opens in the one a bare /studio opens in.
            None => Self {
                path: path.to_string(),
                mode: StudioMode::for_path(path).unwrap_or_else(|| Self::first_example().mode),
                text: None,
            },
        }
    }

    fn of(spec: &'static ExampleSpec) -> Self {
        Self {
            path: spec.vfs_path.to_string(),
            // Every shipped example names its mode (`every_example_opens_in_its_declared_mode`).
            mode: StudioMode::for_file(spec.vfs_path, spec.source).unwrap_or(StudioMode::Code),
            text: Some(spec.source),
        }
    }

    /// The text `mode`'s editor starts with: the opening's text in its own mode's editor — Code and
    /// Web share one — and nothing in the others.
    pub fn editor_text(&self, mode: StudioMode) -> String {
        let shared = |m: StudioMode| matches!(m, StudioMode::Code | StudioMode::Web);
        if mode == self.mode || (shared(mode) && shared(self.mode)) {
            self.text.unwrap_or_default().to_string()
        } else {
            String::new()
        }
    }

    /// The file the Studio shows open from its first frame: the opening's, when the site carries its
    /// text. A reader's own file is open only once the file system has read it.
    pub fn current_file(&self) -> Option<String> {
        self.text.map(|_| self.path.clone())
    }

    /// What the file system's answer changes: `open` is the file open now, `editor` what the
    /// opening's editor holds, and `read` the file's text, `None` when the file system has no such
    /// file.
    ///
    /// What the reader does is theirs: once they have opened another file, or the editor holds
    /// anything but what the Studio opened with, nothing replaces it. Otherwise a reader's file
    /// opens with its text, an example whose stored copy differs opens with that copy, and a file
    /// nobody has falls back to the first example. An example already on screen stays as it is,
    /// stored or not.
    pub fn arrival(&self, open: Option<&str>, editor: &str, read: Option<String>) -> Arrival {
        if open != self.current_file().as_deref() || editor != self.text.unwrap_or_default() {
            return Arrival::Keep;
        }
        match read {
            Some(text) if self.text == Some(text.as_str()) => Arrival::Keep,
            Some(text) => Arrival::Open(text),
            None if self.text.is_some() => Arrival::Keep,
            None => Arrival::Fallback(Self::first_example()),
        }
    }
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/studio_opening/tests.rs"]
mod tests;
