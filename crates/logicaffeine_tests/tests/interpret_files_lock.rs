//! `largo run --interpret` READS AND WRITES THE FILES `largo run` DOES.
//!
//! A file statement (`Write … to file …`, `Read … from file …`) reaches the disk from a compiled program, a path
//! resolving against the directory the program runs in. The interpreter is another way to run the same program,
//! so it reaches the same files by the same paths: each row runs on the interpreter and as the compiled binary,
//! each in a fresh project, and both print the same lines and leave the same files behind. A program agents run
//! through `largo mcp` is a different matter: its sandbox keeps file statements off the disk (`mcp_e2e`'s
//! `the_sandbox_leaves_the_disk_as_it_was`).
#![cfg(not(target_arch = "wasm32"))]

mod cli_common;

use cli_common::{largo, scaffold};
use std::path::{Path, PathBuf};

/// A file statement program: its source, what it prints, and the files it leaves, as `(path, contents)`.
struct Row {
    name: &'static str,
    src: &'static str,
    shows: &'static [&'static str],
    leaves: &'static [(&'static str, &'static str)],
    /// Files the project holds before the run, as `(path, contents)`.
    given: &'static [(&'static str, &'static str)],
}

const ROWS: &[Row] = &[
    Row {
        name: "write_then_read_back",
        src: "## Main\nWrite \"saved\" to file \"out.txt\".\nRead back from file \"out.txt\".\nShow back.\n",
        shows: &["saved"],
        leaves: &[("out.txt", "saved")],
        given: &[],
    },
    Row {
        name: "a_path_into_a_directory_the_run_makes",
        src: "## Main\nWrite \"deep\" to file \"data/notes/a.txt\".\nRead back from file \"data/notes/a.txt\".\n\
              Show back.\n",
        shows: &["deep"],
        leaves: &[("data/notes/a.txt", "deep")],
        given: &[],
    },
    Row {
        name: "a_file_the_project_already_holds",
        src: "## Main\nRead text from file \"input.txt\".\nShow text.\nWrite text + \"!\" to file \"input.txt\".\n",
        shows: &["given"],
        leaves: &[("input.txt", "given!")],
        given: &[("input.txt", "given")],
    },
    Row {
        name: "a_task_writes_the_file_main_reads",
        src: "## To save:\n    Write \"from a task\" to file \"task.txt\".\n\n## Main\nLet h be Launch a task to save.\n\
              Await h into r.\nRead back from file \"task.txt\".\nShow back.\n",
        shows: &["from a task"],
        leaves: &[("task.txt", "from a task")],
        given: &[],
    },
];

fn project(root: &Path, name: &str, row: &Row) -> PathBuf {
    let dir = root.join(name);
    scaffold(&dir, name);
    std::fs::write(dir.join("src/main.lg"), row.src).unwrap();
    for (path, contents) in row.given {
        std::fs::write(dir.join(path), contents).unwrap();
    }
    dir
}

/// What `args` printed in `dir`, or why it failed.
fn shows(dir: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    let out = largo().current_dir(dir).args(args).output().expect("largo should spawn");
    if !out.status.success() {
        return Err(format!(
            "exit {:?}: {}{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect())
}

/// Every row on `args`, each in its own fresh project (`slug` names it): the lines and the files it leaves.
fn every_row_on(args: &[&str], tier: &str, slug: &str) {
    let root = tempfile::tempdir().unwrap();
    let mut wrong = Vec::new();
    for row in ROWS {
        let dir = project(root.path(), &format!("{}_{slug}", row.name), row);
        match shows(&dir, args) {
            Ok(lines) if lines == row.shows => {}
            Ok(lines) => wrong.push(format!("{tier} on `{}` printed {lines:?}, expected {:?}", row.name, row.shows)),
            Err(why) => wrong.push(format!("{tier} on `{}` failed: {why}", row.name)),
        }
        for (path, contents) in row.leaves {
            match std::fs::read_to_string(dir.join(path)) {
                Ok(found) if found == *contents => {}
                Ok(found) => wrong.push(format!("{tier} on `{}` left {path} holding {found:?}, expected {contents:?}", row.name)),
                Err(e) => wrong.push(format!("{tier} on `{}` left no {path}: {e}", row.name)),
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn the_interpreter_reads_and_writes_the_files_the_program_names() {
    every_row_on(&["run", "--interpret"], "the interpreter", "interpreted");
}

#[test]
fn the_tree_walker_reads_and_writes_the_files_the_program_names() {
    every_row_on(&["run", "--interpret", "--engine", "tree"], "the tree-walker", "tree");
}

#[test]
fn the_compiled_program_reads_and_writes_the_same_files() {
    every_row_on(&["-q", "run"], "the compiled program", "compiled");
}
