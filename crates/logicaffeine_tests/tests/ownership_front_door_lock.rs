//! ONE OWNERSHIP VERDICT AT EVERY FRONT DOOR. Whether a program uses a value after giving it away is
//! a property of the program as written, so every way into the toolchain answers it the same way and
//! before any tier runs: `largo check`, `largo run --interpret` on each engine, `largo build`, and
//! `largo emit rust`/`c`/`wasm` all refuse an ill-formed program with the ownership diagnostic, and a
//! well-formed one passes them all. A tier that ran the program instead would print values the
//! language says no longer belong to it; a backend that refused it in its own words would blame the
//! compiler for the program's mistake.
#![cfg(not(target_arch = "wasm32"))]

mod cli_common;

use cli_common::{largo_in, scaffold, stderr, stdout, strip_ansi};
use std::path::Path;

/// Every front door, as the `largo` arguments that take it.
const DOORS: &[&[&str]] = &[
    &["check"],
    &["run", "--interpret", "--engine", "tree"],
    &["run", "--interpret", "--engine", "vm"],
    &["run", "--interpret", "--engine", "jit"],
    &["build"],
    &["emit", "rust"],
    &["emit", "c"],
    &["build", "--emit", "wasm"],
];

/// Programs that use a value after giving it away, each with the words its diagnostic must say.
const ILL_FORMED: &[(&str, &str, &str)] = &[
    (
        "give_then_show",
        "## To keep (xs: Seq of Int):\n    Show length of xs.\n\n\
         ## Main\nLet items be [1, 2, 3].\nGive items to keep.\nShow items.\n",
        "Cannot use 'items' after giving it away",
    ),
    (
        "give_as_a_call_argument_then_show",
        "## To keep (xs: Seq of Int):\n    Show length of xs.\n\n\
         ## Main\nLet items be [1, 2, 3].\nCall keep with give items.\nShow items.\n",
        "Cannot use 'items' after giving it away",
    ),
    (
        "give_in_one_branch_then_show",
        "## To keep (xs: Seq of Int):\n    Show length of xs.\n\n\
         ## Main\nLet items be [1, 2, 3].\nLet n be length of items.\nIf n is greater than 2:\n    \
         Give items to keep.\nShow items.\n",
        "Cannot use 'items' - it might have been given away in a conditional branch",
    ),
];

/// A program that gives a value away and never uses it again: every door accepts it.
const WELL_FORMED: &str = "## To keep (xs: Seq of Int):\n    Show length of xs.\n\n\
     ## Main\nLet items be [1, 2, 3].\nShow items.\nGive items to keep.\n";

fn project(root: &Path, name: &str, source: &str) -> std::path::PathBuf {
    let dir = root.join(name);
    scaffold(&dir, name);
    std::fs::write(dir.join("src/main.lg"), source).unwrap();
    dir
}

#[test]
fn every_front_door_refuses_a_use_after_give_with_the_ownership_diagnostic() {
    let root = tempfile::tempdir().unwrap();
    let mut wrong = Vec::new();
    for (name, source, says) in ILL_FORMED {
        let dir = project(root.path(), name, source);
        for door in DOORS {
            let out = largo_in(&dir, door);
            let text = strip_ansi(&format!("{}{}", stdout(&out), stderr(&out)));
            if out.status.success() || !text.contains(says) || text.contains("compiler bug") {
                wrong.push(format!(
                    "`largo {}` on `{name}`: exit {:?}, expected a refusal saying {says:?}; got:\n{}",
                    door.join(" "),
                    out.status.code(),
                    text.lines().take(8).collect::<Vec<_>>().join("\n")
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n\n"));
}

#[test]
fn every_front_door_accepts_a_give_that_is_never_used_again() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path(), "give_last", WELL_FORMED);
    let mut wrong = Vec::new();
    for door in DOORS {
        let out = largo_in(&dir, door);
        if !out.status.success() {
            wrong.push(format!(
                "`largo {}` refused a well-formed program: {}",
                door.join(" "),
                strip_ansi(&stderr(&out)).lines().take(6).collect::<Vec<_>>().join("\n")
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n\n"));
}
