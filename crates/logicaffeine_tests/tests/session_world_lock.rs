//! ★ LOCK — a tree-walker session on its own stack runs in its caller's world. ★
//!
//! A session runs on a thread of its own ([`on_session_stack`]) because the tree-walker's call
//! depth needs a stack an ordinary thread does not have. The page world — the document, the
//! page's journal, the page clock, the sealed test world, storage — is kept in
//! thread-locals, so a second thread would split it in two: a session would not see what its caller
//! set up, and its caller would not see what the session left. That split is invisible until
//! something reads across it, and then it reads an empty world and reports nothing. So the world
//! goes WITH the session: it moves to the session's thread when the session starts and back when it
//! ends — a session on its own stack is observationally the session run in place.

use logicaffeine_compile::interpreter::on_session_stack;
use logicaffeine_compile::semantics::world::{self, JournalEntry};
use logicaffeine_compile::semantics::{dom, storage};

fn entry(action: &str) -> JournalEntry {
    JournalEntry {
        instance: "/App".to_string(),
        action: action.to_string(),
        changes: Vec::new(),
        failed: None,
    }
}

#[test]
fn a_session_reads_the_world_its_caller_set_up() {
    dom::reset();
    world::reset();
    storage::set(storage::LOCAL, "greeting", "hello").unwrap();
    dom::set_title("before");
    world::journal_push(entry("arrived"));
    let seen = on_session_stack(|| {
        (
            storage::get(storage::LOCAL, "greeting").unwrap(),
            dom::title(),
            world::journal()
                .into_iter()
                .map(|e| e.action)
                .collect::<Vec<_>>(),
        )
    });
    assert_eq!(
        seen.0.as_deref(),
        Some("hello"),
        "the session reads the storage its caller holds"
    );
    assert_eq!(seen.1, "before", "and the document");
    assert_eq!(seen.2, ["arrived"], "and the journal");
}

#[test]
fn the_world_a_session_leaves_is_the_callers_to_read() {
    dom::reset();
    world::reset();
    on_session_stack(|| {
        storage::set(storage::LOCAL, "left", "by the session").unwrap();
        dom::set_title("after");
        world::journal_push(entry("clicked"));
    });
    assert_eq!(
        storage::get(storage::LOCAL, "left").unwrap().as_deref(),
        Some("by the session")
    );
    assert_eq!(dom::title(), "after");
    assert_eq!(
        world::journal()
            .into_iter()
            .map(|e| e.action)
            .collect::<Vec<_>>(),
        ["clicked"]
    );
}

#[test]
fn a_session_inside_a_session_is_the_same_world() {
    world::reset();
    on_session_stack(|| {
        world::journal_push(entry("outer"));
        on_session_stack(|| world::journal_push(entry("inner")));
        assert_eq!(
            world::journal()
                .into_iter()
                .map(|e| e.action)
                .collect::<Vec<_>>(),
            ["outer", "inner"],
            "a nested session runs where it is"
        );
    });
    assert_eq!(world::journal().len(), 2);
}

#[test]
fn a_session_that_panics_still_hands_back_its_world_and_its_panic() {
    world::reset();
    let caught = std::panic::catch_unwind(|| {
        on_session_stack(|| {
            world::journal_push(entry("before the panic"));
            panic!("the session's own panic");
        })
    });
    let panic = caught.expect_err("the panic reaches the caller");
    assert_eq!(
        panic.downcast_ref::<&str>().copied(),
        Some("the session's own panic"),
        "unchanged"
    );
    assert_eq!(
        world::journal()
            .into_iter()
            .map(|e| e.action)
            .collect::<Vec<_>>(),
        ["before the panic"],
        "what the session did before it panicked is the caller's to read"
    );
}

/// The case the split hid: a test suite runs its page as a session, and the journal of the turns it
/// took is read afterwards.
#[test]
fn a_test_runs_journal_is_where_its_caller_reads_it() {
    let src = "\
## Component App
State:
    Let n be 0.
Actions:
    To add:
        Set n to n + 1.
View:
    <button id=\"go\" on-click={add}>{n}</button>

## Test: two clicks
    Mount App.
    Click \"#go\".
    Click \"#go\".
    Expect that the text of \"#go\" is \"2\".
";
    dom::reset();
    let report = logicaffeine_compile::testrun::run_tests(
        src,
        &logicaffeine_compile::testrun::TestConfig::default(),
    )
    .expect("the suite parses");
    assert_eq!(
        report.outcomes[0].verdict,
        logicaffeine_compile::testrun::Verdict::Passed,
        "{:?}",
        report.outcomes[0].failures
    );
    assert_eq!(
        world::journal()
            .into_iter()
            .map(|e| e.action)
            .collect::<Vec<_>>(),
        ["add", "add"],
        "the two turns the test took are in the journal its caller reads"
    );
}

/// The world is where the session is and nowhere else: a thread the session starts has a world of
/// its own, empty, and nothing the session holds reaches it — a thread takes only `Send` data, which
/// the world's `Rc`s are not, so the one way page state crosses a thread is the session's handoff
/// (census below).
#[test]
fn a_thread_the_session_starts_has_none_of_its_world() {
    world::reset();
    let (in_session, in_its_thread) = on_session_stack(|| {
        world::journal_push(entry("in the session"));
        let in_its_thread = std::thread::scope(|scope| {
            scope
                .spawn(|| world::journal().len())
                .join()
                .expect("the thread runs")
        });
        (world::journal().len(), in_its_thread)
    });
    assert_eq!(in_session, 1, "the session holds its world");
    assert_eq!(in_its_thread, 0, "a thread it starts sees none of it");
    assert_eq!(world::journal().len(), 1, "and the caller has it back");
}

/// ★ THE ONE THREAD CROSSING FOR THE PAGE WORLD. The world holds `Rc`s, which the type system keeps
/// on one thread: nothing that holds one can be sent to another thread without an `unsafe impl Send`.
/// So the census of those impls IS the list of ways world state could reach another thread, and the
/// session's handoff is the only one that carries it — a private type holding the world by value,
/// used by `on_session_stack` alone. A new unsafe crossing fails here until it names what it carries.
#[test]
fn the_session_handoff_is_the_only_unsafe_thread_crossing_that_carries_page_state() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut found: Vec<String> = Vec::new();
    let mut stack = vec![root.join("crates"), root.join("apps")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if path.is_dir() {
                if !matches!(name, "target" | "fuzz" | "node_modules") && !name.starts_with('.') {
                    stack.push(path);
                }
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let src = std::fs::read_to_string(&path).unwrap_or_default();
            for line in src.lines() {
                let code = line.split("//").next().unwrap_or("");
                if let Some(at) = code.find("unsafe impl ") {
                    let rest = &code[at + "unsafe impl ".len()..];
                    if rest.starts_with("Send for ") || rest.starts_with("Sync for ") {
                        let head = rest.split('{').next().unwrap_or(rest).trim();
                        found.push(format!("{name} {head}"));
                    }
                }
            }
        }
    }
    found.sort();
    assert_eq!(
        found,
        [
            "aot_tier.rs Send for AotNativeFn",
            "aot_tier.rs Sync for AotNativeFn",
            "interpreter.rs Send for WorldHandoff",
            "lib.rs Send for JitPage",
            "lib.rs Sync for JitPage",
        ],
        "every `unsafe impl Send`/`Sync` in the workspace: a native function pointer (`AotNativeFn`), an \
         executable memory page (`JitPage`), and the session's page-world handoff (`WorldHandoff`) — the \
         only one that carries page state across threads"
    );
    let interpreter =
        std::fs::read_to_string(root.join("crates/logicaffeine_compile/src/interpreter.rs"))
            .unwrap();
    assert!(
        !interpreter.contains("pub struct WorldHandoff")
            && interpreter.contains("struct WorldHandoff(PageWorld);"),
        "the handoff is private and holds the world BY VALUE — no reference crosses the boundary"
    );
    let session = interpreter
        .split("pub fn on_session_stack")
        .nth(1)
        .and_then(|rest| rest.split("\n}\n").next())
        .expect("on_session_stack is defined");
    let uses_in_session = session.matches("WorldHandoff(").count();
    let uses_anywhere = interpreter.matches("WorldHandoff(").count();
    assert_eq!(
        uses_anywhere - uses_in_session,
        1,
        "`WorldHandoff(` appears once outside `on_session_stack` — its own definition — and nowhere else"
    );
}
