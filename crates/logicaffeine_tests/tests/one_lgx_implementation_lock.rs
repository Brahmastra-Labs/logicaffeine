//! ★ LOCK — an LGX page has ONE implementation: its lowered LOGOS. ★
//!
//! A component used to run two ways. The tree-walker carried its own component runtime — instances,
//! turns, triggers, timers, stores, routes, a view renderer — and every other tier ran the page the
//! lowering compiles it to. Two implementations of one language disagree wherever nobody has looked
//! yet, and the ledger of their disagreements (`lgx_known_divergences.txt`) was the measure of it.
//! Once every tier runs the lowered page and the reference is the lowered page on the tree-walker,
//! the second implementation is deleted, and this lock keeps it deleted: it reads the SOURCE for the
//! runtime's entry points, its module, its seat and its tier, so a second implementation cannot
//! return under a new caller without failing here first.

use std::path::PathBuf;

fn compile_src(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../logicaffeine_compile/src")
        .join(rel)
}

fn read(rel: &str) -> String {
    let path = compile_src(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Source with every `//` comment removed, so a doc comment naming a deleted function is not the
/// function.
fn code(src: &str) -> String {
    src.lines()
        .map(|line| match line.find("//") {
            Some(at) => &line[..at],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The component runtime's own entry points in the tree-walker: mounting, settling and rendering a
/// component from its AST, running a turn, and a store's record and actions.
const RUNTIME_ENTRY_POINTS: &[&str] = &[
    "fn mount_component(",
    "fn settle_page(",
    "fn render_root(",
    "fn render_view_nodes(",
    "fn run_turn(",
    "fn run_turn_inner(",
    "fn ensure_store(",
    "fn call_store_action(",
    "fn store_record_key(",
    "fn store_storage(",
    "fn names_a_store(",
];

#[test]
fn the_tree_walker_has_no_component_runtime_of_its_own() {
    let interpreter = code(&read("interpreter.rs"));
    let present: Vec<&str> = RUNTIME_ENTRY_POINTS
        .iter()
        .copied()
        .filter(|f| interpreter.contains(f))
        .collect();
    assert!(
        present.is_empty(),
        "interpreter.rs still carries the AST component runtime: {present:?}. A page runs as its lowered \
         LOGOS on every tier; the tree-walker hosts it (`Interpreter::host_lowered_page`) and runs no \
         component of its own."
    );
}

/// `Replay these actions:` is a driving statement like any other: the tree-walker hands it to the
/// page (`Drive::Replay`) and replays nothing itself.
#[test]
fn replaying_the_journal_is_the_pages_to_do() {
    let interpreter = code(&read("interpreter.rs"));
    let body = interpreter
        .split("fn replay_actions(")
        .nth(1)
        .and_then(|rest| rest.split("\n    }\n").next())
        .expect("`Replay these actions:` has its one execution site");
    assert!(
        body.contains("self.drive_page(crate::page_host::Drive::Replay"),
        "the replay is driven into the page:\n{body}"
    );
    assert!(
        statements(body) <= 2,
        "and nothing else happens there:\n{body}"
    );
}

/// The statements of the function body `text` holds after its signature, however they are laid out: each `;`
/// at the body's own depth ends one, and an expression after the last is one more.
fn statements(text: &str) -> usize {
    let body = text.split_once('{').map_or("", |(_, b)| b);
    let (mut depth, mut count, mut tail) = (0i32, 0, false);
    for c in body.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ';' if depth == 0 => {
                count += 1;
                tail = false;
                continue;
            }
            _ => {}
        }
        if depth >= 0 && !c.is_whitespace() {
            tail = true;
        }
    }
    count + usize::from(tail)
}

#[test]
fn the_page_is_never_seated_in_the_tree_walker_itself() {
    let interpreter = code(&read("interpreter.rs"));
    assert!(
        !interpreter.contains("PageSeat::Own"),
        "a page seated in the tree-walker's own runtime (`PageSeat::Own`) is the second implementation"
    );
}

#[test]
fn the_component_runtime_module_is_gone() {
    assert!(
        !compile_src("semantics/component.rs").exists(),
        "semantics/component.rs is the AST component runtime's state; the page's journal and failure \
         live with the page host"
    );
}

#[test]
fn no_page_tier_runs_the_ast() {
    for rel in ["page_host/mod.rs", "page_host/scenario.rs"] {
        let src = code(&read(rel));
        for tier in ["PageTier::Ast", "PageTierKind::Ast"] {
            assert!(
                !src.contains(tier),
                "{rel} still has a tier that runs the AST component runtime: `{tier}`"
            );
        }
    }
}

#[test]
fn the_host_reconciler_module_is_gone() {
    // A page reconciles in `vdom.lg`, which every tier runs; a host-side reconciler in Rust would
    // be a second implementation of the diff that only a host could execute.
    assert!(
        !compile_src("web_host/reconcile.rs").exists(),
        "web_host/reconcile.rs is a host-side reconciler; the page's own `vdom.lg` is the one"
    );
    let web_host = code(&read("web_host.rs"));
    assert!(
        !web_host.contains("mod reconcile"),
        "web_host declares a reconcile module again"
    );
}

// ---- The catalogue: every second implementation the collapse retires, gone or going. ----

/// A second implementation's name, as the scan matches it: a name ending in `_` is a family of
/// compiler-written names (any identifier it begins); a name with `::` is a path; any other is one
/// whole identifier.
struct Retired {
    name: &'static str,
    why: &'static str,
}

/// Retired and GONE: no crate's or app's `src/`, and not `runtime.js`, names them. One that came back
/// would be a second implementation of something the language now does one way.
const GONE: &[Retired] = &[
    Retired {
        name: "lower_view_html",
        why: "a `-> View` function drawn to HTML by a walk of its own; a server draws the view the lowering \
              compiles, onto the tape, and serializes it",
    },
    Retired {
        name: "lower_component_html",
        why: "a component drawn to HTML by a walk of its own; a server renders the page's one view walk",
    },
    Retired {
        name: "web_host::reconcile",
        why: "a host-side reconciler in Rust; the page's own `vdom.lg` is the one reconciler",
    },
    Retired {
        name: "begin_paint",
        why: "the host-side staging pass a paint opened; a paint's boundary is the commit that emits its stream",
    },
    Retired {
        name: "move_children",
        why: "the holder `{children}` was drawn into and moved out of; a slot's children are a tape range \
              (`vdRangeOpen`/`vdSlot`) the commit places, and the `moveChildren` spelling is refused \
              (`builtins::RETIRED_BUILTINS`)",
    },
];

/// Retired and still GOING: each is in `src/` today, with the reason it is to go. The ledger only
/// shrinks — when the last use of a name leaves `src/` its entry moves to `GONE`, and a new entry is
/// never added here.
const PENDING: &[Retired] = &[
    Retired {
        name: "__SITE_",
        why: "the per-instance address book of vdom site ids, a site made on first draw and kept for the \
              instance's life; vdom resolves a fragment's own sites itself",
    },
    Retired {
        name: "__SEEN_",
        why: "the live-instance registry kept beside vdom's gone detection (`vdGone`) for fan-out — timers, \
              store watchers, relay and window listeners, `Disappears`; fragments tagged by component and key \
              replace it",
    },
    Retired { name: "__KEYS_", why: "the iterable twin of `__SEEN_`; it goes with it" },
    Retired {
        name: "__LGX_PASS",
        why: "the paint-pass counter of the sweep before vdom; the instance stamps against it are read by \
              nothing",
    },
    Retired {
        name: "__DIRS_",
        why: "the pass each directive occurrence was last drawn in, which the `When removed:` sweep compares \
              to `__LGX_PASS` — a second gone detector beside `vdGone`; directive anchors reported through \
              vdom's free list replace it",
    },
    Retired {
        name: "__LISTENER_",
        why: "the action table `__lgx_dispatch_node` reads — a slot per node and event, its spills, its held \
              and left lookups — a second table keyed by the same node and event as vdom's listener pool, which \
              carries the handlers instead",
    },
    Retired {
        name: "repaint_target",
        why: "a wrapper's repaint taken through the root because its slot range is drawn in the caller's \
              scope — a second repaint path; the wrapper's own repaint keeps the caller's range (`vdKeep`)",
    },
];

/// How long `PENDING` may be: the ledger only shrinks, so this only goes down.
const PENDING_AT_MOST: usize = 7;

/// Every source the catalogue reads, as `(path, code)`: each crate's and app's `src/**/*.rs` and the
/// browser host `runtime.js`, with `//` comments removed (a comment naming a retired name is prose).
fn catalogue_sources() -> Vec<(String, String)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut dirs: Vec<PathBuf> = Vec::new();
    for group in ["crates", "apps"] {
        let listed = std::fs::read_dir(root.join(group))
            .unwrap_or_else(|e| panic!("cannot list {group}: {e}"));
        for member in listed.filter_map(|e| e.ok().map(|e| e.path())) {
            let src = member.join("src");
            if src.is_dir() {
                dirs.push(src);
            }
        }
    }
    let mut out = Vec::new();
    while let Some(dir) = dirs.pop() {
        for entry in
            std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("cannot list {}: {e}", dir.display()))
        {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().and_then(|x| x.to_str()) == Some("rs") {
                let text = std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
                out.push((path.display().to_string(), code(&text)));
            }
        }
    }
    let runtime = root.join("crates/logicaffeine_compile/assets/web/runtime.js");
    out.push((
        runtime.display().to_string(),
        code(&std::fs::read_to_string(&runtime).expect("runtime.js")),
    ));
    assert!(
        out.len() > 100,
        "the catalogue read only {} sources",
        out.len()
    );
    out
}

/// Where `name` is named in `sources`, as `path:line`.
fn named_at(name: &str, sources: &[(String, String)]) -> Vec<String> {
    let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut at = Vec::new();
    for (path, src) in sources {
        for (i, line) in src.lines().enumerate() {
            let hit = line.match_indices(name).any(|(k, _)| {
                let before = line[..k].chars().next_back();
                let after = line[k + name.len()..].chars().next();
                let starts = !before.is_some_and(ident) || name.contains("::");
                let ends = name.ends_with('_') || name.contains("::") || !after.is_some_and(ident);
                starts && ends
            });
            if hit {
                at.push(format!("{path}:{}", i + 1));
            }
        }
    }
    at
}

#[test]
fn no_retired_implementation_comes_back() {
    let sources = catalogue_sources();
    for r in GONE {
        let at = named_at(r.name, &sources);
        assert!(
            at.is_empty(),
            "`{}` is retired ({}), and is named again:\n{}",
            r.name,
            r.why,
            at.join("\n")
        );
    }
}

#[test]
fn every_pending_retirement_is_still_pending() {
    let sources = catalogue_sources();
    for r in PENDING {
        assert!(
            !named_at(r.name, &sources).is_empty(),
            "`{}` is gone from every source: move its entry from PENDING to GONE ({})",
            r.name,
            r.why
        );
    }
}

#[test]
fn the_pending_ledger_only_shrinks() {
    assert!(
        PENDING.len() <= PENDING_AT_MOST,
        "PENDING grew to {}; a second implementation is never added to it",
        PENDING.len()
    );
    for r in PENDING.iter().chain(GONE) {
        assert!(!r.why.is_empty(), "`{}` carries its reason", r.name);
    }
    for p in PENDING {
        assert!(
            !GONE.iter().any(|g| g.name == p.name),
            "`{}` is both gone and pending",
            p.name
        );
    }
}

#[test]
fn the_catalogue_sees_a_retired_name_and_not_its_neighbours() {
    let planted = vec![(
        "planted.rs".to_string(),
        code(
            "fn lower_view_html() {}\n\
             // lower_component_html was here\n\
             let s = format!(\"__SITE_{c}\");\n\
             let t = my_begin_painter();\n\
             use crate::web_host::reconcile;\n",
        ),
    )];
    assert_eq!(
        named_at("lower_view_html", &planted),
        ["planted.rs:1"],
        "a whole identifier"
    );
    assert!(
        named_at("lower_component_html", &planted).is_empty(),
        "a comment is prose"
    );
    assert_eq!(
        named_at("__SITE_", &planted),
        ["planted.rs:3"],
        "a family, inside the string that writes it"
    );
    assert!(
        named_at("begin_paint", &planted).is_empty(),
        "a longer identifier is not the name"
    );
    assert_eq!(
        named_at("web_host::reconcile", &planted),
        ["planted.rs:5"],
        "a path"
    );
}
