//! Every statement a block reads is findable by identity afterwards
//! (`Parser::nested_stmt_spans`), which is what lets a diagnostic underline
//! the sentence at fault inside a function, a loop or a branch. That holds only
//! while every block reads its statements through the one path that records
//! their spans (and settles their moves) — `read_block_statement` into a
//! `StmtList`, allocated by `alloc_read_block` — and every block reaches the
//! arena through `alloc_block`. A new construct that reads with a bare
//! `parse_statement()` loop, or allocates a statement slice behind
//! `alloc_block`'s back, would silently lose its spans; this lock turns that
//! into a red test instead.

use std::path::{Path, PathBuf};

fn parser_sources() -> Vec<(PathBuf, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_language/src/parser");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("the parser source directory")
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .map(|path| {
            let text = std::fs::read_to_string(&path).expect("a parser source file");
            (path, text)
        })
        .collect();
    files.sort();
    assert!(files.len() > 5, "found the parser's source files in {}", dir.display());
    files
}

/// Each line containing `needle`, with the name of the function it sits in.
fn sites<'s>(text: &'s str, needle: &str) -> Vec<(usize, Option<&'s str>)> {
    let mut current_fn = None;
    let mut found = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if let Some(name) = fn_name(line) {
            current_fn = Some(name);
        }
        if line.contains(needle) && fn_name(line).is_none() {
            found.push((index + 1, current_fn));
        }
    }
    found
}

fn fn_name(line: &str) -> Option<&str> {
    let rest = line.trim_start();
    let rest = rest.strip_prefix("pub(super) ").or_else(|| rest.strip_prefix("pub(crate) ")).or_else(|| rest.strip_prefix("pub ")).unwrap_or(rest);
    let rest = rest.strip_prefix("fn ")?;
    let end = rest.find(|c: char| !(c.is_alphanumeric() || c == '_'))?;
    Some(&rest[..end])
}

#[test]
fn blocks_read_their_statements_only_through_read_block_statement() {
    // `parse_moving_statement` is the one caller of `parse_statement` (it settles
    // what each statement moved), and only the two readers call it: a block's
    // `read_block_statement`, and the top level, which records into the
    // positional `stmt_spans` table instead.
    let mut strays = Vec::new();
    let (mut settled, mut read_in_blocks, mut read_at_top) = (0, 0, 0);
    for (path, text) in parser_sources() {
        for (line, in_fn) in sites(&text, "self.parse_statement()") {
            match in_fn {
                Some("parse_moving_statement") => settled += 1,
                other => strays.push(format!("{}:{line} parse_statement (in {other:?})", path.display())),
            }
        }
        for (line, in_fn) in sites(&text, "self.parse_moving_statement()") {
            match in_fn {
                Some("read_block_statement") => read_in_blocks += 1,
                Some("parse_program") => read_at_top += 1,
                other => strays.push(format!("{}:{line} parse_moving_statement (in {other:?})", path.display())),
            }
        }
    }
    assert_eq!((settled, read_in_blocks, read_at_top), (1, 1, 1), "the one reading path, found where it lives");
    assert!(
        strays.is_empty(),
        "these read statements around the path that records their spans and settles their \
         moves — read them with `read_block_statement` into a `StmtList` and allocate it with \
         `alloc_read_block`:\n{}",
        strays.join("\n"),
    );
}

#[test]
fn statement_blocks_reach_the_arena_only_through_alloc_block() {
    let mut strays = Vec::new();
    let mut through_the_path = 0;
    for (path, text) in parser_sources() {
        for (line, in_fn) in sites(&text, "expect(\"imperative arenas not initialized\")") {
            match in_fn {
                Some("alloc_block") => through_the_path += 1,
                other => strays.push(format!("{}:{line} (in {other:?})", path.display())),
            }
        }
    }
    assert_eq!(through_the_path, 1, "alloc_block is the one raw statement-slice allocation");
    assert!(
        strays.is_empty(),
        "these allocate a statement block directly — use `alloc_block` for statements the \
         parser writes itself, `alloc_read_block` for statements read from the source:\n{}",
        strays.join("\n"),
    );
}
