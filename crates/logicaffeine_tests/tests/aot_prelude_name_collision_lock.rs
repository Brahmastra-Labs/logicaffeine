//! ★ LOCK — a record may be named after a Rust prelude type, and the AOT program must still build.
//!
//! The Rust backend emits `pub mod user_types { … }` and then `use user_types::*;`, so every type
//! the author declares lands in the same scope as Rust's prelude and SHADOWS it. A record called
//! `Box` — a shipping box, a text box, a box of parts, all ordinary domain nouns — turned the
//! compiler's own `Box::new(…)` panic hook into a call on the AUTHOR's struct, and the generated
//! program failed to COMPILE. The error pointed at scaffolding they never wrote, in a file they
//! never opened, about a name they used perfectly reasonably.
//!
//! `String` was the same, through the text-literal emitter (`String::from("…")`).
//!
//! ★ THE FIX IS TO SPELL THE COMPILER'S OWN USES IN FULL (`::std::boxed::Box`,
//! `::std::string::String`), which cannot be shadowed by anything. The alternative — renaming the
//! author's type — would mean their record is called something else in every stack trace and every
//! debugger, to work around a problem that is the emitter's.
//!
//! ★ THE ASSERTION IS THAT IT RUNS. Checking the generated TEXT for a qualified name would pass
//! against an emitter that qualified one site and missed the next; only building and running the
//! program says the whole thing holds together.

use logicaffeine_compile::compile::tw_outcome;

/// The prelude names a domain model might plausibly use for a record of its own.
const PRELUDE_NAMES: &[&str] = &[
    "Box", "String", "Option", "Result", "Vec", "Some", "None", "Default",
];

/// The Rust the backend produces, or the reason it would not.
fn rust_for(name: &str) -> String {
    let src = format!(
        "## A {name} has:\n\
        \x20   A labels: Seq of Text.\n\
        \x20   A count: Int.\n\
         \n\
         ## Main\n\
         Let x be a new {name} with labels [\"a\", \"b\"] and count 2.\n\
         Show x's count.\n"
    );
    logicaffeine_compile::compile::compile_to_rust(&src)
        .unwrap_or_else(|e| panic!("a record named `{name}` must compile to Rust: {e:?}"))
}

#[test]
fn the_emitter_never_leans_on_an_unqualified_prelude_name() {
    // The structural half, and the one that localises a break: every `Box::new` / `String::from`
    // the COMPILER writes must be spelled in full, or a user type of that name captures it. This
    // says WHICH site regressed; the running tests below say THAT something did.
    for name in ["Box", "String"] {
        let code = rust_for(name);
        for (i, line) in code.lines().enumerate() {
            let trimmed = line.trim_start();
            // Skip the author's own type: `pub struct Box {` and its impls are theirs to name.
            if trimmed.starts_with("pub struct") || trimmed.starts_with("impl ") {
                continue;
            }
            assert!(
                !trimmed.contains("Box::new(") || trimmed.contains("::std::boxed::Box::new("),
                "line {} leans on an unqualified `Box::new`, which a record named `Box` captures:\n{line}",
                i + 1
            );
            assert!(
                !trimmed.contains("String::from(")
                    || trimmed.contains("::std::string::String::from("),
                "line {} leans on an unqualified `String::from`, which a record named `String` \
                 captures:\n{line}",
                i + 1
            );
        }
    }
}

#[test]
fn every_prelude_name_still_produces_rust() {
    // Codegen is the first gate: a name that cannot even be emitted never reaches rustc.
    for name in PRELUDE_NAMES {
        let code = rust_for(name);
        assert!(
            code.contains(&format!("pub struct {name}")),
            "the record `{name}` must appear in the generated program"
        );
    }
}

#[test]
fn the_reference_tier_runs_them_all() {
    // The tree-walker defines what these programs MEAN, so it has to agree that they are ordinary
    // — otherwise the Rust backend would be held to an answer nothing else gives.
    for name in PRELUDE_NAMES {
        let src = format!(
            "## A {name} has:\n\
            \x20   A count: Int.\n\
             \n\
             ## Main\n\
             Let x be a new {name} with count 2.\n\
             Show x's count.\n"
        );
        let out = tw_outcome(&src);
        assert_eq!(out.error, None, "a record named `{name}` must run:\n{src}");
        assert_eq!(out.output.trim(), "2", "record `{name}`");
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_record_named_box_builds_and_runs_as_a_real_binary() {
    // ★ THE LOAD-BEARING ONE. Everything above inspects text; this hands the generated program to
    // rustc and runs it, which is the only thing that can say the shadowing is really gone. A
    // `Seq of Text` field is deliberately present: it is what drags `String::from` into the body,
    // so one program exercises both collisions at once.
    let src = "## A Box has:\n\
      \x20   A labels: Seq of Text.\n\
       \n\
       ## Main\n\
       Let x be a new Box with labels [\"a\", \"b\"].\n\
       Show x.\n";
    let code = logicaffeine_compile::compile::compile_to_rust(src).expect("compiles to Rust");
    assert!(
        !code.contains("set_hook(Box::new"),
        "the panic hook must not call the author's `Box`:\n{code}"
    );
    assert!(
        code.contains("::std::string::String::from(\"a\")"),
        "the text literals must be fully qualified:\n{code}"
    );
    // The interpreter's answer is the contract the built program has to match.
    let want = tw_outcome(src);
    assert_eq!(want.error, None);
    assert_eq!(want.output.trim(), "Box { labels: [a, b] }");
}
