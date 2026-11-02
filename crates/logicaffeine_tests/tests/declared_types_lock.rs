//! ★ LOCK — every declared type names the op that defines its value, through every pass that moves ops. ★
//!
//! `CompiledProgram::declared_types` records what the program DECLARES a value to be (`a new Seq of
//! Text`, `a new Map of Text to Int`, `Let x: T be …`, a call to a function declaring `-> Seq of Todo`),
//! keyed by the absolute pc of the op that defines the value. A statically-typed backend takes the
//! value's kind from it. Keyed by op position, it is only as good as every pass that moves ops: an entry
//! left behind by a deletion names a DIFFERENT op, and a backend then types the wrong value — a refusal
//! at best, a miscompile at worst.
//!
//! Three halves: every entry the compiler records names a defining op of the right shape; the peephole
//! (the one pass that deletes ops before the typed backends read the program) keeps every surviving
//! entry on the same op, defining the same register; and a source census — every function in the
//! compile crate that rebuilds a program's op vector also remaps `declared_types`.

use logicaffeine_compile::compile::compile_to_program;
use logicaffeine_compile::vm::{BoundaryType, CompiledProgram, Op};
use std::path::Path;

/// A program with declarations in Main, in a function after a register-shuffling loop the peephole
/// thins, and bound from a call to a function that declares its result.
const PROGRAM: &str = "\
## A Todo has:
    a text, which is Text.

## To words(n: Int) -> Seq of Text:
    Let out be a new Seq of Text.
    Let mutable i be 0.
    While i < n:
        Push \"w\" to out.
        Set i to i + 1.
    Return out.

## To shuffle(n: Int) -> Int:
    Let mutable a be 1.
    Let mutable b be 2.
    Let mutable i be 0.
    While i < n:
        Let t be a.
        Set a to b.
        Set b to t.
        Set i to i + 1.
    Let names be a new Seq of Text.
    Let counts be a new Map of Text to Int.
    Let tags be a new Set of Text.
    Let todos be a new Seq of Todo.
    Push \"x\" to names.
    Set item \"x\" of counts to 1.
    Add \"t\" to tags.
    Return a + length of names + length of counts + length of tags + length of todos.

## Main
Let xs be a new Seq of Int.
Let w be words(2).
Let annotated: Seq of Text be words(1).
Show shuffle(3).
Show length of xs + length of w + length of annotated.
";

fn compiled() -> CompiledProgram {
    compile_to_program(PROGRAM).unwrap_or_else(|e| panic!("the fixture compiles: {e:?}"))
}

/// The register an op defines, for the op shapes a declaration may sit on.
fn defined(op: &Op) -> Option<u16> {
    match *op {
        Op::NewEmptyList { dst }
        | Op::NewEmptyListWithCapacity { dst, .. }
        | Op::NewEmptyListFixed { dst, .. }
        | Op::NewEmptyListI32 { dst }
        | Op::NewEmptySet { dst }
        | Op::NewEmptyMap { dst }
        | Op::NewEmptyMapWithCapacity { dst, .. }
        | Op::NewList { dst, .. }
        | Op::Move { dst, .. } => Some(dst),
        _ => None,
    }
}

/// Each declaration as `(op name, defined register, type)`, failing on an entry whose op defines no
/// value of a declarable shape.
fn resolved(program: &CompiledProgram) -> Vec<(&'static str, u16, BoundaryType)> {
    program
        .declared_types
        .iter()
        .map(|(pc, bt)| {
            let op = program
                .code
                .get(*pc as usize)
                .unwrap_or_else(|| panic!("declaration at pc {pc} is past the code"));
            let dst = defined(op).unwrap_or_else(|| {
                panic!("declaration {bt:?} at pc {pc} names {op:?}, which defines no value")
            });
            (op.name(), dst, bt.clone())
        })
        .collect()
}

#[test]
fn every_declaration_the_compiler_records_names_the_op_that_defines_its_value() {
    let program = compiled();
    let found = resolved(&program);
    let text = || BoundaryType::Text;
    for (want, what) in [
        (
            BoundaryType::Seq(Box::new(BoundaryType::Int)),
            "`a new Seq of Int` in Main",
        ),
        (
            BoundaryType::Seq(Box::new(text())),
            "`a new Seq of Text`, or the declared result of `words`",
        ),
        (
            BoundaryType::Map(Box::new(text()), Box::new(BoundaryType::Int)),
            "`a new Map of Text to Int`",
        ),
        (BoundaryType::Set(Box::new(text())), "`a new Set of Text`"),
        (
            BoundaryType::Seq(Box::new(BoundaryType::Struct("Todo".to_string()))),
            "`a new Seq of Todo`",
        ),
    ] {
        assert!(
            found.iter().any(|(_, _, bt)| *bt == want),
            "no declaration recorded for {what}: {found:?}"
        );
    }
    assert!(
        found.iter().filter(|(name, _, bt)| *name == "Move" && *bt == BoundaryType::Seq(Box::new(text()))).count() >= 2,
        "a `Let` of a call to `words` (declared `-> Seq of Text`), annotated or not, declares the Move that binds it: {found:?}"
    );
    assert!(
        program.declared_types.windows(2).all(|w| w[0].0 <= w[1].0),
        "declarations are sorted by pc"
    );
}

#[test]
fn the_peephole_keeps_every_surviving_declaration_on_its_op() {
    let before = compiled();
    let mut after = before.clone();
    logicaffeine_compile::vm::peephole::peephole(&mut after);
    // The precondition this test stands on: the peephole deleted ops AHEAD of declarations, so every
    // later declaration had to move.
    let last = |p: &CompiledProgram| {
        p.declared_types
            .last()
            .map(|(pc, _)| *pc)
            .expect("the fixture declares values")
    };
    assert!(
        last(&after) < last(&before),
        "the peephole deleted nothing ahead of a declaration — the fixture lost its point"
    );
    let (was, now) = (resolved(&before), resolved(&after));
    // What survives is the same declarations, in order, each still on an op of the same shape defining
    // the same register; the peephole deletes only dead moves, so only a `Move`'s declaration may drop.
    let mut rest = now.iter().peekable();
    for entry in &was {
        if rest.peek() == Some(&entry) {
            rest.next();
        } else {
            assert_eq!(
                entry.0, "Move",
                "a declaration that did not survive the peephole must be a dead move's: {entry:?}"
            );
        }
    }
    assert!(
        rest.next().is_none(),
        "the peephole left declarations the program did not have:\n{now:?}\nvs\n{was:?}"
    );
    assert!(
        now.iter()
            .any(|(_, _, bt)| *bt == BoundaryType::Set(Box::new(BoundaryType::Text))),
        "the declarations after the shuffle loop survive the peephole: {now:?}"
    );
}

/// Every function in the compile crate that REBUILDS a program's op vector — assigns it, drains it,
/// inserts into it, removes from it — must also remap `declared_types`, or the table rots silently.
#[test]
fn every_pass_that_moves_ops_remaps_the_declared_types() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_compile/src");
    let mut files = Vec::new();
    let mut stack = vec![src];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
            .flatten()
        {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().and_then(|x| x.to_str()) == Some("rs") {
                files.push(p);
            }
        }
    }
    assert!(
        files.len() > 50,
        "the source walk found only {} files",
        files.len()
    );
    let moves = |line: &str| {
        let l = line.trim_start();
        !l.starts_with("//")
            && ([
                ".code.drain(",
                ".code.insert(",
                ".code.remove(",
                ".code.retain(",
                ".code.splice(",
                ".code.truncate(",
            ]
            .iter()
            .any(|m| l.contains(m))
                || l.split(".code =")
                    .nth(1)
                    .is_some_and(|rest| !rest.starts_with('=')))
    };
    let mut found = 0;
    let mut missing = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap_or_default();
        let mut starts: Vec<usize> = Vec::new();
        for (i, line) in text.lines().enumerate() {
            let l = line.trim_start();
            if l.starts_with("fn ") || l.starts_with("pub fn ") || l.starts_with("pub(crate) fn ") {
                starts.push(i);
            }
        }
        let lines: Vec<&str> = text.lines().collect();
        for (k, &start) in starts.iter().enumerate() {
            let end = starts.get(k + 1).copied().unwrap_or(lines.len());
            let body = &lines[start..end];
            if body.iter().any(|l| moves(l)) {
                found += 1;
                if !body.iter().any(|l| l.contains("declared_types")) {
                    missing.push(format!("{}: {}", file.display(), lines[start].trim()));
                }
            }
        }
    }
    assert!(
        found >= 1,
        "the census found no op-moving pass at all — the reader is broken"
    );
    assert!(
        missing.is_empty(),
        "passes that move a program's ops without remapping `declared_types`:\n{}",
        missing.join("\n")
    );
}
