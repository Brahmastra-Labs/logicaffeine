//! ★ LOCK — every `Actions:` verb either REACHES THE BROWSER or is refused BY NAME. ★
//!
//! `Actions:` is where a page does things, and `vm/compiler.rs`'s `Stmt::Ui(_)` catch-all meant
//! most of what it can say **failed the build for the browser**: `Toggle done.`,
//! `Set the title to "Todos".`, `Navigate to "/todos".`, `Focus "#draft".`, `Save draft as "…".`
//! and the rest. `lgx_lower` lowers exactly two of the twenty-five `UiStmt` variants
//! (`Mount`, `CallStore`); the driving verbs are handled in the VM compiler; everything else fell
//! through.
//!
//! ★ AND THE REFUSAL NAMED NOTHING — not the verb, not the page:
//!
//! ```text
//! LGX routes and the remaining action verbs are not executable on this tier yet
//! ```
//!
//! ★ WHY IT WAS INVISIBLE. Every other tier says yes. They parse (`ui_verbs_lock` is green), they
//! run on the tree-walker, they pass `largo test`, and `lgx_vm_parity_lock`'s `NOT_YET_ON_THE_VM`
//! catalogue did not carry a single one of them — so nothing anywhere recorded them as a tier gap.
//! `docs/web.md` teaches one of them by name. A page written from the documentation did not build.
//!
//! ★ THE SHAPE OF THIS LOCK IS THE POINT. The gap existed because **nothing enumerated the verbs**
//! — each was added where it was needed and none was ever counted. So this file is a CATALOGUE
//! over all 25 `UiStmt` variants, and the exhaustiveness test fails when a new one appears. A verb
//! is `Lowered` (it reaches the browser) or `RefusedByName` (it does not, and says so, naming
//! itself). "Silently absent" is not one of the options.

use logicaffeine_compile::compile::compile_to_wasm;

/// What a verb is allowed to be on the bytecode/browser tier.
#[derive(PartialEq, Debug, Clone, Copy)]
enum Tier {
    /// Reaches the browser: the page compiles to a wasm module.
    Lowered,
    /// Does not reach the browser yet — and the refusal NAMES the verb, so the author learns which
    /// sentence to change rather than that "action verbs" in general are unsupported.
    RefusedByName,
    /// Blocked by a DIFFERENT gap, so its own verb cannot even be measured. Recorded rather than
    /// hidden: a row that fails for someone else's reason is worse than no row, because it reads
    /// as evidence about this verb.
    BlockedByAnotherGap(&'static str),
}

/// Every `UiStmt` variant, the sentence that writes it, and where it stands.
///
/// The sentence goes in an `Actions:` body, which is the position that mattered and the one that
/// was never checked — `ui_verbs_lock` proves these PARSE, on a tier with no build step.
/// ★ THE SPLIT IS THE HOST ABI, and it is worth reading before adding a row.
///
/// Six of these need NO new browser import — `navigate`, `replaceRoute`, `storageSet`,
/// `storageGet`, `storageRemove` are already in the frozen host surface, and `Toggle` touches no
/// document at all (it is `Set <field> to not <field>`, pure state). Those are marked `Lowered`
/// and are what a real page most needs: persistence and navigation.
///
/// The other eleven want a host function that does not exist yet (`focus`, `scrollTo`, a title
/// setter, the clipboard, notifications, history-back, reload). Adding one is a DELIBERATE act —
/// `wasm_host_abi_lock` freezes the list, and a new import touches the wasm import table, the
/// usage scan, the browser shim, `runtime.js`, the node shim and every wasmi test host, because a
/// module importing something its host lacks does not degrade, it FAILS TO INSTANTIATE. So they
/// are `RefusedByName` — and the refusal now quotes the author's own sentence.
const VERBS: &[(&str, &str, &str, Tier)] = &[
    // ★ ONE IMPORT FOR THE ELEMENT-COMMAND FAMILY (`dom_element_command`). The command is
    // performed and then the event a browser would have raised is delivered through the SAME
    // dispatcher a click goes through — so a `When "#draft" is focused:` sentence answers on every
    // tier. In a real browser the platform raises it too; the delivery is for the tiers with no
    // platform. Neither requires an answer: a form that focuses its own first input answers nothing.
    ("Focus", "Focus \"#draft\".", "", Tier::Lowered),
    // Blur and Hover joined the vocabulary so every event a handler can WAIT FOR can be CAUSED —
    // `blurred` and `hovered` could be declared and driven by nothing. Both refuse on the browser
    // tier for the same reason Focus does.
    ("Blur", "Blur \"#draft\".", "", Tier::Lowered),
    // ★ THE SAME SENTENCE MEANS TWO THINGS. From a TEST, `Hover "#card".` is "the visitor's
    // pointer went here" and an element answering nothing means the test named the wrong one —
    // the rule `Click` follows. From inside an ACTION it is a page raising `mouseover` on its own
    // element, which nothing need be listening for: a page cannot hover over itself, so demanding
    // an answer there would refuse the only sensible reading. Where it is written says which.
    ("Hover", "Hover \"#draft\".", "", Tier::Lowered),
    // ★ BOTH HALVES WERE MISSING — neither had an implementation on ANY tier, so a page writing
    // one was refused everywhere rather than merely in a browser. They ride the element-command
    // import, and they are recorder STATE rather than ops: scrolling and selecting are things a
    // browser does TO a document that stays the same, so there is nothing for an op-log to hold.
    ("ScrollTo", "Scroll to \"#bottom\".", "", Tier::Lowered),
    // ★ THE RULE IS COMPILED LOGOS, THE IMPORT IS A READ. `Add class`/`Remove class` need to know
    // which class WORDS survive — a decision — so that lives in web.lg's `classListWith` /
    // `classListWithout` where every tier runs it, and the only new host function is the generic
    // `dom_get_attribute`. Two host functions named `dom_add_class`/`dom_remove_class` would have
    // been fewer lines and would have put the rule in a .js file only a browser executes.
    (
        "AddClass",
        "Add class \"shake\" to \"#card\".",
        "",
        Tier::Lowered,
    ),
    (
        "RemoveClass",
        "Remove class \"shake\" from \"#card\".",
        "",
        Tier::Lowered,
    ),
    (
        "SelectText",
        "Select the text in \"#draft\".",
        "",
        Tier::Lowered,
    ),
    // ★ THREE CAPABILITIES, THREE IMPORTS — not one `pageCommand`. They have three permission
    // models and three ways to be denied: a clipboard write a browser may refuse, a file the viewer
    // may decline, a notification that needs consent. On a tier with no browser each RECORDS that
    // the page asked, which is the part a test needs and the only part honest to claim.
    (
        "CopyToClipboard",
        "Copy \"hello\" to the clipboard.",
        "",
        Tier::Lowered,
    ),
    ("Navigate", "Navigate to \"/todos\".", "", Tier::Lowered),
    (
        "ReplaceUrl",
        "Replace the URL with \"/todos/1\".",
        "",
        Tier::Lowered,
    ),
    // ★ BOTH HALVES WERE MISSING — the reference tier had no implementation either, because the
    // recorder logged what a page DID (`route_ops`, replaceState included) and never modelled
    // where Back would LAND. That is a different question, so it got its own stack.
    ("GoBack", "Go back.", "", Tier::Lowered),
    // ★ TAUGHT BY NAME IN `docs/web.md` AND UNBUILDABLE — the worst pairing there is: the docs
    // hand someone the sentence and the browser tier refuses the whole component the moment they
    // write it. One import (`dom_set_title`), one builtin, no decision moved into JavaScript.
    ("SetTitle", "Set the title to \"Todos\".", "", Tier::Lowered),
    // ★ IT WAS THE ORDER, NOT A MISSING MECHANISM. This row read `RefusedByName` for a real
    // reason: a turn writes its state back at the END, so `Reload the page.` cleared every instance
    // mid-body and the write-back then re-created the one it had just cleared, carrying the values
    // the reload existed to forget — the reference tier ended with an empty document and the
    // bytecode tier with the page still there.
    //
    // What closed it was the deferral focus and blur already needed, one step further: the reload
    // runs AFTER the write-back, and the paint is skipped, because a page that has been torn down
    // has nothing to draw. No new mechanism, no decision about virtual documents — the sentence
    // simply had to happen last.
    ("Reload", "Reload the page.", "", Tier::Lowered),
    ("Notify", "Notify \"Saved.\".", "", Tier::Lowered),
    // ★ IT NEVER HAD A ROW, BECAUSE THE SENTENCE DID NOT EXIST. `When a message arrives on "t":`
    // reached the browser and nothing could SEND one — the inbound half of a construct shipped
    // without its outbound half. That is invisible to a census over verbs: this catalogue reports
    // whether each verb the language HAS reaches a browser, and a verb the language lacks has
    // nothing to report. The exhaustiveness that guards a `UiStmt` variant cannot see a missing
    // one, which is the hole a paired census over CAPABILITIES would close.
    ("Publish", "Publish \"hi\" on \"chat\".", "", Tier::Lowered),
    ("Toggle", "Toggle done.", "", Tier::Lowered),
    ("Save", "Save draft as \"draft-cache\".", "", Tier::Lowered),
    (
        "Load",
        "Load \"draft-cache\" into draft.",
        "",
        Tier::Lowered,
    ),
    ("Forget", "Forget \"draft-cache\".", "", Tier::Lowered),
    (
        "Download",
        "Download draft as \"out.txt\".",
        "",
        Tier::Lowered,
    ),
    // ★ ITS `BlockedByAnotherGap` ROW CAME TRUE, and the lock is what said so: it waited on the
    // timer TRIGGERS, and the moment those landed this went red demanding a real row. The verb
    // needed no import of its own — a timer is a due time held per instance on the page's own
    // clock, so stopping one is writing the value that means "not running" and asking the host to
    // wake the page for what still runs (`wakeAfter`); no `clearInterval` crosses the ABI.
    (
        "StopTimer",
        "Stop the timer refresh.",
        "\x20   Every 60 seconds as refresh:\n\x20       Set draft to draft.\n",
        Tier::Lowered,
    ),
    // S15: the same timer armed afresh from now — its period read from its sentence as the turn
    // holds the state, the arming a first render does.
    (
        "StartTimer",
        "Start the timer refresh.",
        "\x20   Every 60 seconds as refresh:\n\x20       Set draft to draft.\n",
        Tier::Lowered,
    ),
    // Already lowered before this file existed — kept so the catalogue is the whole surface.
    (
        "CallStore",
        "Call Session's login with \"ada\".",
        "STORE",
        Tier::Lowered,
    ),
];

/// The variants a page never writes in an `Actions:` body — `Mount` is a program's entry and the
/// driving verbs belong to a `## Test`. They are listed so the exhaustiveness check below can be a
/// real partition of all 25 rather than a subset nobody audited.
const NOT_IN_ACTIONS: &[&str] =
    // `Raise` joins the drivers: it is how a TEST causes something that happens to the page
    // (going offline, the window resizing), not something a page's `Actions:` does to itself.
    &[
        "Mount",
        "Click",
        "Submit",
        "Choose",
        "Press",
        "TypeInto",
        "RaiseWindowEvent",
    ];

/// A page whose single action runs `verb`, with the state and elements the sentence needs.
fn page_with(verb: &str, deps: &str) -> String {
    // No blank line inside a component's sections — a blank line closes the block, which is what
    // the first version of this fixture tripped over. `deps` carries only what a particular verb
    // needs, so one row's requirement cannot make every other row red.
    let store = if deps == "STORE" {
        "## Store Session\nState:\n\x20   Let name be \"guest\".\nActions:\n\x20   To login (who: Text):\n\x20       Set name to who.\n\n"
    } else {
        ""
    };
    let uses = if deps == "STORE" {
        "Use Session.\n"
    } else {
        ""
    };
    let extra = if deps == "STORE" { "" } else { deps };
    format!(
        "{store}## Component App\n\
         {uses}State:\n\
        \x20   Let done be false.\n\
        \x20   Let draft be \"\".\n\
         Actions:\n\
        \x20   To go:\n\
        \x20       {verb}\n\
         {extra}View:\n\
        \x20   <div id=\"card\"><input id=\"draft\" type=\"text\"><p id=\"bottom\">x</p><button id=\"b\" on-click={{go}}>go</button></div>\n\n\
         ## Main\n\
         Mount App.\n"
    )
}

/// The fixture must be a page that COMPILES with no verb in it, or every row below is red for a
/// reason that has nothing to do with its verb — which is exactly how the first draft of this file
/// reported seventeen tier gaps that were one malformed blank line.
#[test]
fn the_fixture_itself_compiles() {
    let src = page_with("Set draft to draft.", "");
    if let Err(e) = compile_to_wasm(&src) {
        panic!("the control page must reach the browser, or no row here means anything.\nERROR: {e:?}\n\n{src}");
    }
}

#[test]
fn every_action_verb_reaches_the_browser_or_names_itself() {
    let mut wrong: Vec<String> = Vec::new();
    for (name, sentence, deps, tier) in VERBS {
        let src = page_with(sentence, deps);
        match (compile_to_wasm(&src), tier) {
            (Ok(bytes), Tier::Lowered) => {
                assert_eq!(
                    &bytes[..4],
                    b"\0asm",
                    "`{name}` compiled to something that is not a module"
                );
            }
            (Err(e), Tier::Lowered) => {
                wrong.push(format!(
                    "{name}: expected to reach the browser, refused: {e:?}"
                ));
            }
            (Err(e), Tier::RefusedByName) => {
                // ★ THE HALF THAT MATTERS EVEN WHEN A VERB IS NOT LOWERED. A refusal that says
                // "the remaining action verbs" sends the reader to search their whole page.
                let msg = format!("{e:?}").to_lowercase();
                if !msg.contains(&name.to_lowercase()) && !msg.contains(&first_word(sentence)) {
                    wrong.push(format!("`{name}` is refused without naming itself: {e:?}"));
                }
            }
            (Ok(_), Tier::RefusedByName) => {
                wrong.push(format!(
                    "`{name}` now reaches the browser — move it to `Tier::Lowered`"
                ));
            }
            (Err(_), Tier::BlockedByAnotherGap(_)) => {}
            (Ok(_), Tier::BlockedByAnotherGap(why)) => wrong.push(format!(
                "`{name}` compiles now — `{why}` is closed, so give it a real row"
            )),
        }
    }
    assert!(
        wrong.is_empty(),
        "action verbs on the browser tier:\n{}",
        wrong.join("\n")
    );
}

fn first_word(sentence: &str) -> String {
    sentence
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_lowercase()
}

#[test]
fn the_catalogue_covers_every_ui_statement() {
    // ★ THE EXHAUSTIVENESS CHECK, and the reason this file is a catalogue. The verbs were never
    // counted, so nobody could see that seventeen of them did not build. A new `UiStmt` variant
    // must be classified here — reaching the browser, refusing by name, or explicitly not
    // something an `Actions:` body writes.
    let ast = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../logicaffeine_language/src/ast/stmt.rs"),
    )
    .expect("the view/UI AST");
    let body = ast
        .split("pub enum UiStmt<'a> {")
        .nth(1)
        .and_then(|s| s.split("\n}").next())
        .expect("UiStmt is declared there");
    let declared: Vec<String> = body
        .lines()
        .filter_map(|l| {
            let t = l.trim();
            let head = t
                .split(|c: char| c == '{' || c == ',' || c == '(')
                .next()?
                .trim();
            (!head.is_empty()
                && head.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                && head.chars().all(|c| c.is_ascii_alphanumeric()))
            .then(|| head.to_string())
        })
        .collect();

    let known: Vec<&str> = VERBS
        .iter()
        .map(|(n, ..)| *n)
        .chain(NOT_IN_ACTIONS.iter().copied())
        .collect();
    let missing: Vec<&String> = declared
        .iter()
        .filter(|d| !known.contains(&d.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "these `UiStmt` variants are in no catalogue row: {missing:?}\n\
         Add each to VERBS (with `Lowered` if it compiles for the browser, `RefusedByName` if it \
         does not yet and says so) or to NOT_IN_ACTIONS. A verb nobody counted is how seventeen of \
         them came to fail the build while every other tier said yes."
    );
    let stale: Vec<&&str> = known
        .iter()
        .filter(|k| !declared.contains(&k.to_string()))
        .collect();
    assert!(
        stale.is_empty(),
        "the catalogue names variants that no longer exist: {stale:?}"
    );
    assert!(
        declared.len() >= 25,
        "the UI statement surface shrank to {}",
        declared.len()
    );
}

/// ★ `UiStmt::role()` (`ast/stmt.rs`) IS THE SAME PARTITION THIS FILE PROVES BY COMPILING PAGES —
/// cross-checked here against TWO independent sources of truth read from their own text, so the
/// two catalogues cannot silently drift: this file's own `VERBS` (every verb `role()` calls
/// `Action` or `Both` must have a row here, proving it compiles inside `Actions:`) and
/// `NOT_IN_ACTIONS` (every verb `role()` calls `Drive`-only must be here, proving it never does);
/// and `page_host::Drive`'s own variants (every one must be `Drive` or `Both` in `role()` — it
/// drives a `## Main`/`## Test` statement to a page under test).
#[test]
fn every_verb_role_agrees_with_where_it_actually_runs() {
    let stmt_src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../logicaffeine_language/src/ast/stmt.rs"),
    )
    .expect("the view/UI AST");
    let start = stmt_src
        .find("pub fn role(&self) -> Role {")
        .expect("`UiStmt::role` is declared");
    let rest = &stmt_src[start..];
    let body = &rest[..rest.find("\n    }\n").expect("`role`'s body closes")];

    // `variant [| variant ...] => Role::X,` arms, read as text rather than by calling `role()` on
    // constructed values — a `UiStmt` variant holds arena-allocated `&Expr` fields this file has
    // no arena to build, and the point is to hold the SOURCE of the match to the SOURCE of the two
    // other catalogues, not to exercise the method at runtime.
    //
    // Every `UiStmt::Name` mention in an arm's pattern is found by searching for the marker
    // directly, rather than splitting the pattern on `|` and reading each piece's own leading
    // identifier — a doc comment ahead of an arm's first variant (this function has one per arm)
    // has no `|` before it, so it shares that variant's own split-piece, and the comment's own
    // prose becomes the "identifier" a naive split reads instead of the variant it precedes.
    let mut role_of: std::collections::HashMap<String, &'static str> = Default::default();
    let mut cursor = body;
    while let Some(arrow) = cursor.find("=> Role::") {
        let pattern = &cursor[..arrow];
        let after = &cursor[arrow + "=> Role::".len()..];
        let role = if after.starts_with("Drive") {
            "Drive"
        } else if after.starts_with("Action") {
            "Action"
        } else if after.starts_with("Both") {
            "Both"
        } else {
            panic!("`UiStmt::role` returns an unrecognized `Role` variant at: {after}")
        };
        let mut scan = pattern;
        while let Some(at) = scan.find("UiStmt::") {
            let after_marker = &scan[at + "UiStmt::".len()..];
            let name: String = after_marker
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            scan = &after_marker[name.len()..];
            if !name.is_empty() {
                role_of.insert(name, role);
            }
        }
        let comma = after.find(',').unwrap_or(after.len());
        cursor = &after[comma + 1..];
    }

    let mut wrong = Vec::new();
    for (name, ..) in VERBS {
        match role_of.get(*name) {
            Some(&"Drive") => wrong.push(format!(
                "`{name}` compiles inside `Actions:` (this file's own `VERBS` row proves it) but `role()` says Drive-only"
            )),
            Some(_) => {}
            None => wrong.push(format!("`{name}` has no arm in `UiStmt::role`")),
        }
    }
    for name in NOT_IN_ACTIONS {
        match role_of.get(*name) {
            Some(&"Drive") => {}
            Some(other) => wrong.push(format!(
                "`{name}` never compiles inside `Actions:` (`NOT_IN_ACTIONS`) but `role()` says {other}"
            )),
            None => wrong.push(format!("`{name}` has no arm in `UiStmt::role`")),
        }
    }

    // `page_host::Drive`'s own variants, renamed onto their `UiStmt` spelling where the two
    // differ (`Type` → `TypeInto`, `Raise` and `DropFile` → `RaiseWindowEvent`, the statement both
    // are carried out from); its four non-UI members
    // (`AdvanceTime`, `AnswerFetch`, `ServeFetchesWithRoutes`, `Replay`) drive a different `Stmt`
    // kind entirely and have no `UiStmt::role` to check. The exemption checks itself: each name
    // must still be a `Drive` variant, and must still have no `UiStmt::role` arm, or it would be
    // hiding a verb this census owes a check.
    let drive_src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../logicaffeine_compile/src/page_host/mod.rs"),
    )
    .expect("page_host::Drive");
    let dstart = drive_src
        .find("pub enum Drive {")
        .expect("`Drive` is declared");
    let drest = &drive_src[dstart..];
    let dbody = &drest[..drest.find("\n}").expect("`Drive`'s body closes")];
    fn rename(n: &str) -> &str {
        match n {
            "Type" => "TypeInto",
            "Raise" | "DropFile" => "RaiseWindowEvent",
            other => other,
        }
    }
    let non_ui = [
        "AdvanceTime",
        "AnswerFetch",
        "ServeFetchesWithRoutes",
        "Replay",
    ];
    let mut variants: Vec<&str> = Vec::new();
    for line in dbody.lines() {
        let t = line.trim();
        let Some(head) = t.split(|c: char| c == '{' || c == ',').next() else {
            continue;
        };
        let head = head.trim();
        if head.is_empty() || !head.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
            continue;
        }
        variants.push(head);
        if non_ui.contains(&head) {
            continue;
        }
        let name = rename(head);
        match role_of.get(name) {
            Some(&"Action") => wrong.push(format!(
                "`page_host::Drive::{head}` drives `{name}` from a test, but `role()` says Action-only"
            )),
            Some(_) => {}
            None => wrong.push(format!("`{name}` (from `page_host::Drive::{head}`) has no arm in `UiStmt::role`")),
        }
    }
    for name in non_ui {
        if !variants.contains(&name) {
            wrong.push(format!("`{name}` is exempted as a non-UI `page_host::Drive` variant, but `Drive` has no such variant"));
        }
        if let Some(role) = role_of.get(name) {
            wrong.push(format!(
                "`{name}` is exempted as non-UI, but `UiStmt::role` has an arm for it ({role})"
            ));
        }
    }

    assert!(
        wrong.is_empty(),
        "`UiStmt::role()` disagrees with its two sources of truth:\n{}",
        wrong.join("\n")
    );
}

/// ★ A LOWERING THAT COMPILES IS NOT A LOWERING THAT AGREES. Each verb this file moved to
/// `Lowered` has to DO on the bytecode tier what the tree-walker does — a page that builds and
/// then behaves differently in a browser is worse than one that refused.
#[test]
fn the_lowered_verbs_behave_the_same_on_both_tiers() {
    use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
    use logicaffeine_compile::semantics::dom;

    // `Toggle` flips the field, and the flip must survive the turn's write-back.
    let toggle = "## Component App\nState:\n\x20   Let done be false.\n\
        Actions:\n\x20   To go:\n\x20       Toggle done.\n\
        View:\n\x20   <p id=\"d\">{done}</p>\n\x20   <button id=\"b\" on-click={go}>go</button>\n\n\
        ## Main\nMount App.\nClick \"#b\".\n";
    dom::reset();
    let tw = tw_outcome(toggle);
    assert_eq!(tw.error, None, "tree-walker:\n{toggle}");
    let want = dom::render();
    assert!(
        want.contains("true"),
        "the toggle must flip on the reference tier: {want}"
    );
    dom::reset();
    let vm = vm_outcome(toggle);
    assert_eq!(vm.error, None, "VM:\n{toggle}");
    assert_eq!(
        dom::render(),
        want,
        "`Toggle` renders differently on the two tiers"
    );

    // `Navigate to` moves the route, which the page can read back.
    //
    // ★ THIS ASSERTION USED TO BE UNREACHABLE. The view read `{pagePath()}` — a function that
    // exists NOWHERE in the language — so the reference tier answered "Unknown function" every
    // time, and an `if tw.error.is_none()` around the whole comparison meant the failure was the
    // thing that skipped it. A guard that swallows the error it should report turns a test into
    // decoration; the real reader is `routePath()`, and the comparison now always runs.
    let nav = "## Component App\nState:\n\x20   Let n be 0.\n\
        Actions:\n\x20   To go:\n\x20       Navigate to \"/todos\".\n\
        View:\n\x20   <p id=\"p\">{routePath()}</p>\n\x20   <button id=\"b\" on-click={go}>go</button>\n\n\
        ## Main\nMount App.\nClick \"#b\".\n";
    dom::reset();
    let tw = tw_outcome(nav);
    assert_eq!(
        tw.error, None,
        "`Navigate to` must run on the reference tier:\n{nav}"
    );
    let want = dom::render();
    assert!(
        want.contains("/todos"),
        "and the page must read the new route back: {want}"
    );
    dom::reset();
    let vm = vm_outcome(nav);
    assert_eq!(vm.error, None, "`Navigate to` must run on the VM:\n{nav}");
    assert_eq!(
        dom::render(),
        want,
        "`Navigate to` renders differently on the two tiers"
    );
}

/// ★ A REFUSAL MUST NOT CLAIM ANOTHER TIER RENDERS THE PAGE WHEN NONE DOES.
///
/// The browser-tier mount refusal ended, unconditionally, *"The tree-walker renders it; the
/// bytecode tier cannot yet"*. That is true of a TIER GAP — `Every … seconds:` really does run on
/// the reference tier — and false of a sentence NO tier delivers: `When the window resizes:` is
/// refused at mount by the reference tier too, by name. Telling that author their page works on
/// the tree-walker sends them looking for a tier to run it on.
///
/// Both readings now come from one table (`Trigger::undeliverable`), because two tiers have to
/// agree about which sentences exist at all.
#[test]
fn the_refusal_does_not_promise_a_tier_that_cannot_run_it() {
    let page = |trigger: &str| {
        format!(
            "## Component App\nState:\n\x20   Let n be 0.\nActions:\n\x20   {trigger}\n\
             \x20       Set n to n + 1.\nView:\n\x20   <p id=\"n\">{{n}}</p>\n\n## Main\nMount App.\n"
        )
    };

    // ★★ THERE IS NO TIER GAP LEFT TO TEST, AND THAT IS THE RESULT.
    //
    // This half held a verb the reference tier ran and the browser did not, and checked that its
    // refusal said so rather than blaming the author's sentence. The subject moved five times —
    // `Every 60 seconds:`, `When an action fails …:`, `Focus`, `Hover`, `Reload the page.` — and
    // each closure turned this test RED on its way out, which is exactly how a test written
    // against a gap is supposed to behave. `Reload` was the last, and it was the interesting one:
    // it was refused BY CHOICE, because a turn writes its state back at the END and clearing every
    // instance mid-body meant the write-back re-created what it had just cleared. What closed it
    // was the ORDER, not new machinery.
    //
    // Asserting over a gap that no longer exists would be a test that checks nothing while looking
    // like it checks something, so it asserts the achievement instead: every verb the reference
    // tier runs, the browser runs too. The wording rule it used to guard is not lost — it is in
    // `every_action_verb_reaches_the_browser_or_names_itself`, which reads the same catalogue —
    // and the moment a row goes back to `RefusedByName`, that rule has a subject again.
    assert!(
        !VERBS
            .iter()
            .any(|(.., tier)| matches!(tier, Tier::RefusedByName)),
        "a verb is refused again: {:?}. Restore the wording check — a refusal that names the \
         author's sentence must also say the sentence is FINE and the tier is not, or its author \
         goes looking for a mistake they did not make.",
        VERBS
            .iter()
            .filter(|(.., t)| matches!(t, Tier::RefusedByName))
            .map(|(n, ..)| *n)
            .collect::<Vec<_>>()
    );

    // NO TIER AT ALL: the reference tier refuses it too, so the claim would be false.
    //
    // ★ THE EXAMPLE HAS MOVED TWICE, AND THAT IS THE GOOD DIRECTION. It was
    // `When the window resizes:`, then `When a file is dropped on …:`; both now RUN on both tiers,
    // so neither is an example of a sentence nothing delivers. A test whose premise is a gap has
    // to move as the gap closes, or it quietly starts asserting something else — and going RED is
    // how that gets noticed.
    //
    // ★★ AND THE OTHER HALF IS GONE TOO — THIS IS THE DAY IT PREDICTED.
    //
    // The subject here was `When a message arrives on …:`, kept because it waited on a realtime
    // TRANSPORT rather than on a lowering, and the note above it said: "If this ever goes red too,
    // every sentence the language offers reaches a browser and this half of the test has nothing
    // left to check." It went red. The transport turned out to be built —
    // `logicaffeine_system::relay` already carried `Subscribe`/`Publish`/`Event` between a browser
    // tab and a native node — so what looked like a different kind of work was one import.
    //
    // So this asserts the achievement, the same way the verb half above does: NOTHING is
    // undeliverable. Written against the compiler's own table rather than a sample sentence,
    // because a sample would have to be a sentence that fails, and there is no longer one to pick.
    let ast = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../logicaffeine_language/src/ast/component.rs"),
    )
    .expect("the Trigger AST");
    let start = ast
        .find("pub fn undeliverable")
        .expect("`Trigger::undeliverable` must exist");
    let rest = &ast[start..];
    let body = &rest[..rest.find("\n    }").expect("its body must close")];
    assert!(
        !body.contains("Trigger::Message"),
        "`Message` is back in `undeliverable`. If a tier stopped delivering it that is a \
         regression; if the relay wiring was deliberately withdrawn, restore the sample-sentence \
         check above so the refusal's WORDING is guarded again."
    );
    // The page that used to be the sample now RUNS, which is the whole claim.
    let now = page("When a message arrives on \"news\" (body: Text):");
    assert!(
        logicaffeine_compile::compile::tw_outcome(&now)
            .error
            .is_none(),
        "every trigger reaches a tier now, including a message on a topic"
    );
    compile_to_wasm(&now).expect("and it compiles for the browser");
}
