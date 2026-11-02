//! The shape of a clean checkout, enforced.
//!
//! `.config/repo-shape.toml` is the definition: which entries may sit at the repository root, what
//! never belongs in a checkout (bytecode, dependency trees, build output, process residue, empty
//! files), how large a checked-in file may be, how examples and test binaries are named, which
//! scripts are entry points at the top of `scripts/` and which folders hold the rest, that no
//! checked-in text names an absolute home directory, and that no tracked file is also covered by an
//! ignore rule.
//!
//! "A checkout" is every file `git` would carry for this tree: tracked files that exist on disk,
//! plus untracked files no ignore rule covers. So a stray file dropped at the root goes RED here
//! the moment it appears, before anyone commits it. `work/` is outside the product and is not
//! checked.
//!
//! Each rule has a `pending` list naming today's known offenders. A rule is clean when its list is
//! empty. Two things are enforced on the list itself: every row must still be a real offender (a row
//! whose file was cleaned up is stale and must be deleted), and no offender may exist that is not on
//! it. The lists therefore only shrink.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repo root")
        .to_path_buf()
}

struct Shape {
    table: toml::Table,
}

impl Shape {
    fn load() -> Shape {
        let path = repo_root().join(".config/repo-shape.toml");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        Shape {
            table: text
                .parse()
                .unwrap_or_else(|e| panic!("{}: {e}", path.display())),
        }
    }

    fn strings(&self, section: &str, key: &str) -> Vec<String> {
        self.table
            .get(section)
            .and_then(|s| s.get(key))
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| {
                panic!(".config/repo-shape.toml: [{section}] {key} must be an array of strings")
            })
            .iter()
            .map(|v| {
                v.as_str()
                    .unwrap_or_else(|| panic!("[{section}] {key}: every entry is a string"))
                    .to_string()
            })
            .collect()
    }

    fn integer(&self, section: &str, key: &str) -> u64 {
        self.table
            .get(section)
            .and_then(|s| s.get(key))
            .and_then(|v| v.as_integer())
            .unwrap_or_else(|| {
                panic!(".config/repo-shape.toml: [{section}] {key} must be an integer")
            }) as u64
    }

    fn pending(&self, rule: &str) -> BTreeSet<String> {
        self.strings("pending", rule).into_iter().collect()
    }
}

/// Every file a checkout of this tree holds, repo-relative, `work/` excluded.
fn checkout() -> Vec<String> {
    let root = repo_root();
    let out = Command::new("git")
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .current_dir(&root)
        .output()
        .expect("git ls-files");
    assert!(
        out.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut paths: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|p| !p.is_empty() && !p.starts_with("work/"))
        .filter(|p| root.join(p).symlink_metadata().is_ok())
        .map(str::to_string)
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Compares the offenders a rule finds with its `pending` list, and explains both directions.
fn settle(rule: &str, found: BTreeSet<String>, shape: &Shape, what: &str, fix: &str) {
    let pending = shape.pending(rule);
    let new: Vec<&String> = found.difference(&pending).collect();
    let stale: Vec<&String> = pending.difference(&found).collect();
    let mut msg = String::new();
    if !new.is_empty() {
        msg.push_str(&format!(
            "\n{} {what}:\n    {}\n\n{fix}\n",
            new.len(),
            new.iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("\n    ")
        ));
    }
    if !stale.is_empty() {
        msg.push_str(&format!(
            "\n{} row(s) in [pending] {rule} no longer offend — the cleanup landed. Delete them from \
             .config/repo-shape.toml so the list keeps shrinking:\n    {}\n",
            stale.len(),
            stale.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n    ")
        ));
    }
    assert!(msg.is_empty(), "{msg}");
}

#[test]
fn the_root_holds_only_the_allowed_entries() {
    let shape = Shape::load();
    let allowed: BTreeSet<String> = shape.strings("root", "allowed").into_iter().collect();
    let found: BTreeSet<String> = checkout()
        .iter()
        .map(|p| p.split('/').next().unwrap().to_string())
        .filter(|top| !allowed.contains(top))
        .collect();
    settle(
        "root",
        found,
        &shape,
        "root entr(ies) outside [root] allowed",
        "The root is the repository's front page. Product code belongs under crates/ or apps/, guides \
         under docs/, tooling under scripts/ or .config/, and workspace material in work/ (its own \
         repo). Move the entry, or — only if it is a new top-level part of the product — add it to \
         [root] allowed.",
    );
}

#[test]
fn nothing_that_never_belongs_in_a_checkout_is_in_it() {
    let shape = Shape::load();
    let components = shape.strings("never_tracked", "components");
    let suffixes = shape.strings("never_tracked", "suffixes");
    let names = shape.strings("never_tracked", "names");
    let found: BTreeSet<String> = checkout()
        .into_iter()
        .filter(|p| {
            p.split('/').any(|c| components.iter().any(|x| x == c))
                || suffixes.iter().any(|s| p.ends_with(s.as_str()))
                || names.iter().any(|n| n == basename(p))
        })
        .collect();
    settle(
        "never_tracked",
        found,
        &shape,
        "file(s) that never belong in a checkout",
        "Bytecode, dependency trees, build output and process residue are regenerated, not versioned — \
         and compiled bytecode carries its source's text verbatim. Delete the file or `git rm --cached` \
         it; the matching .gitignore rule keeps it out afterwards.",
    );
}

#[test]
fn no_file_is_empty_unless_it_is_a_placeholder() {
    let shape = Shape::load();
    let placeholders = shape.strings("empty", "allowed_names");
    let root = repo_root();
    let found: BTreeSet<String> = checkout()
        .into_iter()
        .filter(|p| !placeholders.iter().any(|n| n == basename(p)))
        .filter(|p| {
            std::fs::metadata(root.join(p))
                .map(|m| m.is_file() && m.len() == 0)
                .unwrap_or(false)
        })
        .collect();
    settle(
        "empty",
        found,
        &shape,
        "empty file(s)",
        "An empty file is either an orphan (a module nothing declares, a trace a tool left behind) or a \
         placeholder. Delete an orphan; a real placeholder is a `.gitkeep`.",
    );
}

#[test]
fn no_file_is_larger_than_the_size_budget() {
    let shape = Shape::load();
    let max = shape.integer("size", "max_bytes");
    let allowed: BTreeSet<String> = shape.strings("size", "allowed").into_iter().collect();
    let root = repo_root();
    let found: BTreeSet<String> = checkout()
        .into_iter()
        .filter(|p| !allowed.contains(p))
        .filter(|p| {
            std::fs::metadata(root.join(p))
                .map(|m| m.len() > max)
                .unwrap_or(false)
        })
        .collect();
    settle(
        "oversize",
        found,
        &shape,
        &format!("file(s) over the {max}-byte budget"),
        "Large checked-in files are almost always regenerable tool output (bitstream dumps, synth logs, \
         ROM tables). Keep the generator and a small receipt, untrack the output, and ignore it. A \
         genuinely hand-made large file goes in [size] allowed with the reason beside it.",
    );
}

#[test]
fn examples_are_named_for_what_they_do_and_say_it() {
    let shape = Shape::load();
    let forbidden = shape.strings("names", "example_forbidden_prefixes");
    let header = shape
        .strings("names", "example_header")
        .pop()
        .expect("[names] example_header");
    let root = repo_root();
    let found: BTreeSet<String> = checkout()
        .into_iter()
        .filter(|p| p.ends_with(".rs") && p.split('/').rev().nth(1) == Some("examples"))
        .filter(|p| {
            forbidden
                .iter()
                .any(|f| basename(p).starts_with(f.as_str()))
                || !std::fs::read_to_string(root.join(p))
                    .map(|t| t.trim_start().starts_with(header.as_str()))
                    .unwrap_or(true)
        })
        .collect();
    settle(
        "examples",
        found,
        &shape,
        "example(s) with a scratch name or no purpose line",
        "An example is part of the crate's front door. Give it a name that says what it does and open \
         it with a `//!` line that says why it exists; a throwaway probe belongs in a scratch directory \
         outside the tree.",
    );
}

#[test]
fn test_binaries_are_not_scratch_named() {
    let shape = Shape::load();
    let forbidden = shape.strings("names", "test_forbidden_prefixes");
    let found: BTreeSet<String> = checkout()
        .into_iter()
        .filter(|p| {
            p.starts_with("crates/logicaffeine_tests/tests/")
                && p.matches('/').count() == 3
                && p.ends_with(".rs")
        })
        .filter(|p| {
            forbidden
                .iter()
                .any(|f| basename(p).starts_with(f.as_str()))
        })
        .collect();
    settle(
        "tests",
        found,
        &shape,
        "test binar(ies) with a scratch name",
        "A test binary's file name is its public name (`-E 'binary(<name>)'`). Name it for the behavior \
         it pins; a diagnostic with no assertion is not a test.",
    );
}

#[test]
fn only_entry_points_sit_at_the_top_of_scripts() {
    let shape = Shape::load();
    let entry: BTreeSet<String> = shape.strings("scripts", "top_level").into_iter().collect();
    let found: BTreeSet<String> = checkout()
        .into_iter()
        .filter_map(|p| p.strip_prefix("scripts/").map(str::to_string))
        .filter(|rest| !rest.contains('/'))
        .filter(|name| !entry.contains(name) && name != "README.md")
        .map(|name| format!("scripts/{name}"))
        .collect();
    settle(
        "scripts",
        found,
        &shape,
        "script(s) at the top of scripts/ that are not entry points",
        "The top of scripts/ holds only what people run (the suite runners, release and packaging \
         checks). Everything else lives in its category folder: test/, bench/, hw/, accel/, lean/, \
         wasm/, web/, release/, dev/.",
    );
}

/// The folders directly below `scripts/` that the checkout holds.
fn script_folders_present() -> BTreeSet<String> {
    checkout()
        .into_iter()
        .filter_map(|p| {
            p.strip_prefix("scripts/")
                .and_then(|rest| rest.split_once('/'))
                .map(|(d, _)| d.to_string())
        })
        .collect()
}

#[test]
fn every_folder_of_scripts_is_a_declared_one() {
    let declared: BTreeSet<String> = Shape::load()
        .strings("scripts", "folders")
        .into_iter()
        .collect();
    let present = script_folders_present();
    let undeclared: Vec<&String> = present.difference(&declared).collect();
    let absent: Vec<&String> = declared.difference(&present).collect();
    assert!(
        undeclared.is_empty() && absent.is_empty(),
        "\nfolder(s) of scripts/ that [scripts] folders does not declare: {undeclared:?}\n\
         declared folder(s) scripts/ does not hold: {absent:?}\n\n\
         A script that is not an entry point lives in the folder for its area. A new area is a new \
         folder, declared in .config/repo-shape.toml and given its line in scripts/README.md.\n"
    );
}

#[test]
fn the_scripts_readme_indexes_every_entry_point_and_folder() {
    let readme = std::fs::read_to_string(repo_root().join("scripts/README.md"))
        .expect("scripts/README.md indexes scripts/");
    let rows: BTreeSet<String> = readme
        .lines()
        .filter_map(|l| l.strip_prefix("| `"))
        .filter_map(|l| l.split_once('`').map(|(name, _)| name.to_string()))
        .collect();
    let mut want: BTreeSet<String> = Shape::load()
        .strings("scripts", "top_level")
        .into_iter()
        .collect();
    want.extend(
        script_folders_present()
            .into_iter()
            .map(|f| format!("{f}/")),
    );
    let missing: Vec<&String> = want.difference(&rows).collect();
    let stale: Vec<&String> = rows.difference(&want).collect();
    assert!(
        missing.is_empty() && stale.is_empty(),
        "scripts/README.md has one table row per entry point (`name`) and per folder (`name/`):\n  \
         missing: {missing:?}\n  rows for what scripts/ does not hold: {stale:?}"
    );
}

/// Ledgers, logs and comments written before 0.11.0 name scripts at the top of `scripts/`, and
/// `scripts/README.md` sends a reader to the file by its name, which holds only while no two scripts share
/// one. `wiki_trace/` is a crate (its files are modules) and `openlane-overlay/` mirrors OpenLane's own
/// layout; neither holds scripts named for a reader.
#[test]
fn no_two_scripts_share_a_file_name() {
    let mut by_name: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for p in checkout() {
        let Some(rest) = p.strip_prefix("scripts/") else {
            continue;
        };
        if rest.starts_with("wiki_trace/") || rest.starts_with("openlane-overlay/") {
            continue;
        }
        let name = rest.rsplit('/').next().unwrap_or(rest).to_string();
        if name != "README.md" {
            by_name.entry(name).or_default().push(p);
        }
    }
    let shared: Vec<String> = by_name
        .into_iter()
        .filter(|(_, ps)| ps.len() > 1)
        .map(|(n, ps)| format!("{n}: {}", ps.join(", ")))
        .collect();
    assert!(
        shared.is_empty(),
        "\nscripts that share a file name:\n    {}\n\nName each for what it does; a reader finds a script by its name.\n",
        shared.join("\n    ")
    );
}

/// Local-only trees (toolchains, model weights, logs, sibling checkouts) are often a SYMLINK into a
/// shared location — a content snapshot of the tree links them rather than copying hundreds of GB.
/// A directory-only ignore rule (`/tools/`) never matches a symlink, so the link would read as an
/// untracked root entry. Asking git about the bare name, without a trailing slash, is exactly the
/// question a symlink poses.
#[test]
fn local_only_trees_are_ignored_as_a_directory_or_a_link() {
    let shape = Shape::load();
    let root = repo_root();
    let unignored: BTreeSet<String> = shape
        .strings("local_only", "names")
        .into_iter()
        .filter(|name| {
            !Command::new("git")
                .args(["check-ignore", "-q", "--no-index", name])
                .current_dir(&root)
                .status()
                .expect("git check-ignore")
                .success()
        })
        .collect();
    assert!(
        unignored.is_empty(),
        "\nlocal-only tree name(s) not ignored as a plain path:\n    {}\n\n\
         Anchor the .gitignore rule without a trailing slash (`/tools`, not `/tools/`): a trailing \
         slash matches only a directory, so a snapshot that links the tree in as a symlink shows it \
         as an untracked root entry.\n",
        unignored.iter().map(String::as_str).collect::<Vec<_>>().join("\n    ")
    );
}

/// Tracked paths some ignore rule covers, from `git ls-files -i -c` with the given exclude arguments.
fn tracked_and_ignored(excludes: &[&str]) -> Vec<String> {
    let out = Command::new("git")
        .args(["ls-files", "-z", "--cached", "--ignored"])
        .args(excludes)
        .current_dir(repo_root())
        .output()
        .expect("git ls-files");
    assert!(
        out.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect()
}

/// An ignore rule untracks nothing: a path already in the index stays there whatever `.gitignore`
/// says, so a rule reading "generated, never committed" can sit above a committed copy of exactly
/// that file, and every clone carries it.
#[test]
fn no_tracked_file_is_also_ignored() {
    let both = tracked_and_ignored(&["--exclude-standard"]);
    assert!(
        both.is_empty(),
        "\n{} tracked path(s) an ignore rule also covers:\n    {}\n\n\
         Untrack a generated or private file (`git rm --cached`), or narrow the rule so it no longer \
         matches a file that belongs in the repository.\n",
        both.len(),
        both.join("\n    ")
    );
}

#[test]
fn the_ignore_detector_sees_a_tracked_file() {
    assert_eq!(
        tracked_and_ignored(&["--exclude=/Cargo.toml"]),
        vec!["Cargo.toml".to_string()]
    );
}

#[test]
fn every_stdlib_module_is_a_logos_source_file() {
    let shape = Shape::load();
    let dir = shape.strings("stdlib", "dir").pop().expect("[stdlib] dir");
    let extension = shape
        .strings("stdlib", "extension")
        .pop()
        .expect("[stdlib] extension");
    let prefix = format!("{dir}/");
    let found: BTreeSet<String> = checkout()
        .into_iter()
        .filter(|p| {
            p.strip_prefix(prefix.as_str())
                .is_some_and(|rest| !rest.contains('/'))
        })
        .filter(|p| !p.ends_with(&format!(".{extension}")))
        .collect();
    settle(
        "stdlib",
        found,
        &shape,
        &format!("stdlib module(s) not ending in .{extension}"),
        "A standard-library module is LOGOS source, so it carries the LOGOS extension. Rename it and \
         update the loader's include_str! and every reader of the old name.",
    );
}

/// `/home/<user>/` or `/Users/<user>/` — an absolute path into somebody's machine. `generic` names
/// the account names that belong to no one (a CI runner's home, a documentation placeholder).
fn names_a_home_directory(text: &str, generic: &[String]) -> bool {
    ["/home/", "/Users/"].iter().any(|prefix| {
        text.match_indices(prefix).any(|(i, _)| {
            let rest = &text[i + prefix.len()..];
            let user: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
                .collect();
            !user.is_empty()
                && rest[user.len()..].starts_with('/')
                && !generic.iter().any(|g| *g == user)
        })
    })
}

#[test]
fn no_checked_in_text_names_a_home_directory() {
    let shape = Shape::load();
    let generic = shape.strings("home_paths", "generic_users");
    let root = repo_root();
    let found: BTreeSet<String> = checkout()
        .into_iter()
        .filter(|p| {
            let Ok(bytes) = std::fs::read(root.join(p)) else {
                return false;
            };
            let head = &bytes[..bytes.len().min(8192)];
            !head.contains(&0) && names_a_home_directory(&String::from_utf8_lossy(&bytes), &generic)
        })
        .collect();
    settle(
        "home_paths",
        found,
        &shape,
        "file(s) naming an absolute home directory",
        "A path into one machine's home directory works only on that machine and names the person who \
         owns it. Resolve from the repository root (scripts/lib/root.sh, CARGO_MANIFEST_DIR) or an \
         environment variable, and regenerate recorded data without the prefix.",
    );
}

#[test]
fn the_home_path_detector_tells_a_home_directory_from_a_mention() {
    let generic = vec!["runner".to_string()];
    assert!(names_a_home_directory(
        "cd /home/alice/logicaffeine",
        &generic
    ));
    assert!(names_a_home_directory(
        "base = '/Users/bob/dev/x'",
        &generic
    ));
    assert!(!names_a_home_directory(
        "files under /home/ are per-user",
        &generic
    ));
    assert!(!names_a_home_directory(
        "the runner's /home/runner/work/ checkout",
        &generic
    ));
    assert!(!names_a_home_directory("see /Users/ on macOS", &generic));
}
