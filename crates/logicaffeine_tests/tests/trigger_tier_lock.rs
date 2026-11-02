//! ★ LOCK — every `Actions:` TRIGGER either reaches the browser or refuses BY NAME. ★
//!
//! `trigger_refusal_lock` already made the refusal readable: it names the sentence the author
//! wrote, through `Trigger::form`, so `Every … seconds:` is reported as the author typed it rather
//! than as `every 60000000000ms`. Its header is also explicit about what it does NOT assert —
//! *"which triggers reach the browser. That set grows as effects land, and a lock pinning it would
//! go stale the way four fixtures did in one day."*
//!
//! ★ THAT IS RIGHT ABOUT A MEMBERSHIP LIST AND WRONG ABOUT A PARTITION, and the difference is the
//! whole reason this file exists beside it rather than instead of it. "These five reach the
//! browser" rots on every landing. "Every variant is classified, and silently-absent is not one of
//! the options" cannot rot — staleness IS the failure it detects, and moving a trigger across is a
//! deliberate one-line edit this test forces you to make. `action_verb_tier_lock` holds exactly
//! that shape over all 25 `UiStmt` verbs, and it is how seventeen non-building verbs were found.
//!
//! ★ THE GAP IT MEASURES, in React's terms: `useState` works everywhere and **`useEffect` works
//! only on the tier you test on**. A stateful component refuses every trigger but an element event,
//! `When the page loads` and `When this component appears` — so a timer, a `When x changes:`, a
//! route change, going offline and scroll-to-bottom are all honest refusals rather than working
//! effects. Honest is a great deal better than silent, and it is not the same as done.

use logicaffeine_compile::compile::compile_to_wasm;

#[derive(PartialEq, Debug, Clone, Copy)]
enum Tier {
    /// The trigger compiles for the browser and its handler runs there.
    ///
    /// ★ "RUNS THERE" MEANS IN A BROWSER, AND FOR THREE ROWS THAT IS THE WHOLE SENTENCE.
    /// `ActionFails` and `RouteChanges` are delivered HOST → MODULE — a trap unwinding to the host,
    /// or `dom_navigate` calling the closure `onRoute` stored — and the in-process bytecode runner
    /// cannot call a function it finds at run time. So both are `Lowered`, both work in a browser,
    /// and neither fires on the tier that `## Test` and `lgx_vm_parity_lock` drive. A reader takes
    /// `Lowered` for "works"; `host_delivered_triggers_lock` measures the difference and says which
    /// rows it applies to.
    Lowered,
    /// It does not reach the browser yet, and the refusal names the author's own sentence.
    RefusedByName,
}

/// Every `Trigger` variant: the sentence that writes it, where it stands, and the FORM its refusal
/// must quote.
///
/// ★ THE FOURTH COLUMN IS `Trigger::form()`'S OWN WORDS, pinned here. A refusal is read by the
/// person who wrote the line, so it names the FORM rather than the values — `When … changes:`, not
/// `every 60000000000ms`, which is true and is not what anybody typed. Writing the expected text
/// out means this file also catches `form()` drifting away from the sentence the parser accepts,
/// which is the one way that message could quietly stop being findable.
const TRIGGERS: &[(&str, &str, Tier, &str)] = &[
    // ---- reaches a browser ----
    (
        "Selector",
        "When \"#b\" is clicked:\n\x20       Set n to n + 1.",
        Tier::Lowered,
        "When \"…\" is …:",
    ),
    // ★ Needed NO host import. A typed fetch already branched on its answer and already built an
    // error-carrying `Result` on both failure paths; both were telling only the action that asked.
    (
        "FetchFails",
        "When a fetch fails (url: Text) and (why: Text):\n\x20       Set draft to why.",
        Tier::Lowered,
        "When a fetch fails …:",
    ),
    (
        "PageLoads",
        "When the page loads:\n\x20       Set n to n + 1.",
        Tier::Lowered,
        "When the page loads:",
    ),
    (
        "Appears",
        "When this component appears:\n\x20       Set n to n + 1.",
        Tier::Lowered,
        "When this component appears:",
    ),
    // `When <field> changes:` — the effect that needed no host import. A turn already knows what
    // it wrote, so the mechanism is a comparison; see `turn_body`.
    (
        "FieldChanges",
        "When n changes:\n\x20       Set draft to \"x\".",
        Tier::Lowered,
        "When … changes:",
    ),
    // ★ THE SAME VARIANT, A DIFFERENT ANSWER — and the reason this row exists.
    //
    // `Trigger::FieldChanges { store: Option<Symbol>, field }` is ONE variant carrying two
    // sentences: `When n changes:` watches this component's own field, and
    // `When Session's name changes:` watches a STORE's. The census below is exhaustive over
    // VARIANTS, so the single row above satisfied it completely — while the store form was refused
    // by name and nothing anywhere said so. The catalogue could report "16/17 reach the browser"
    // with a trigger every store-using page needs sitting outside it, structurally invisible.
    //
    // ★ EXHAUSTIVE OVER VARIANTS IS NOT EXHAUSTIVE OVER BEHAVIOURS. Where a FIELD of a variant
    // changes the answer, the field needs its own row, because `match` cannot notice what it does
    // not split on. Any variant that grows an `Option` deciding a tier belongs here twice.
    (
        "FieldChanges/store",
        "When Session's name changes:\n\x20       Set draft to \"x\".",
        Tier::Lowered,
        // ★ A DIFFERENT FORM STRING FROM THE ROW ABOVE, which is the point: `Trigger::form()` used to
        // return one sentence for both readings, so these two rows were indistinguishable to anything
        // that read the catalogue. Now the fourth column discriminates, and a future divergence
        // between the two — one refused, one not — cannot hide behind a shared description.
        "When <Store>'s … changes:",
    ),
    // ---- refuses, by name ----
    // ★ NO HOST IMPORT. Every component that asks to be told keeps a map from instance key to the
    // PAINT PASS it last rendered in; a paint stamps what it draws, and anything still carrying an
    // older number was not drawn. The page's own retained maps already knew what was on screen.
    (
        "Disappears",
        "When this component disappears:\n\x20       Set n to n + 1.",
        Tier::Lowered,
        "When this component disappears:",
    ),
    // ★ MOVED ACROSS THE PARTITION, DELIBERATELY. `onRoute(handler)` was already a builtin with a
    // host import and a `runtime.js` implementation registering a popstate handler — the whole
    // browser half existed. Only the LGX trigger had never been wired to it, so the language
    // refused a capability the runtime already had.
    (
        "RouteChanges",
        "When the route changes:\n\x20       Set n to n + 1.",
        Tier::Lowered,
        "When the route changes:",
    ),
    (
        "Online",
        "When the app comes back online:\n\x20       Set n to n + 1.",
        Tier::Lowered,
        "When the app comes back online:",
    ),
    (
        "Offline",
        "When the app goes offline:\n\x20       Set n to n + 1.",
        Tier::Lowered,
        "When the app goes offline:",
    ),
    (
        "WindowResizes",
        "When the window resizes:\n\x20       Set n to n + 1.",
        Tier::Lowered,
        "When the window resizes:",
    ),
    // ★ MOVED TO `Lowered` — it listens on the PAGE, and needed no new host import: `body`,
    // `addEventListener` and `eventKey` were already frozen, and `listener_closure` already
    // threaded a key filter for the keyed-selector case. Only the wiring was missing.
    (
        "Key",
        "When the user presses \"Escape\":\n\x20       Set n to n + 1.",
        Tier::Lowered,
        "When the user presses …:",
    ),
    // ★ ONE GEOMETRY READ (`dom_scroll_gap`), and the THRESHOLD stays in compiled LOGOS. What
    // reaches the handler on every tier is the semantic event, `scrolledtobottom`, so a tier with
    // no viewport can cause it and a browser decides which of its many `scroll` events count.
    (
        "ScrollsToBottom",
        "When the user scrolls to the bottom:\n\x20       Set n to n + 1.",
        Tier::Lowered,
        "When the user scrolls to the bottom …:",
    ),
    (
        "Every",
        "Every 60 seconds:\n\x20       Set n to n + 1.",
        Tier::Lowered,
        "Every … seconds:",
    ),
    (
        "After",
        "After 5 seconds:\n\x20       Set n to n + 1.",
        Tier::Lowered,
        "After … seconds:",
    ),
    // ★ NO IMPORT. A trap unwinds to the HOST, which learns only that something failed and has no
    // idea whose turn it was — so the MODULE writes that down before anything can fail, and the
    // dispatch is compiled LOGOS like every other trigger. The host's whole contribution is to
    // call `__lgx_on_failure` with the message it already captured for its panel.
    (
        "ActionFails",
        "When an action fails (why: Text):\n\x20       Set draft to why.",
        Tier::Lowered,
        "When an action fails …:",
    ),
    // ★ THE LAST ONE, AND THE TRANSPORT WAS ALREADY BUILT. This row read `RefusedByName` for the
    // whole campaign, and the reason recorded against it — "needs a WebSocket import AND a replica
    // protocol" — was wrong on both counts. `logicaffeine_system::relay` already carried
    // `Subscribe`/`Publish`/`Event` frames over one wire protocol shared by the native server, the
    // native client and a browser WebSocket client. And the trigger is a SUBSCRIPTION, independent
    // of `## Store … shared on "…"` — that is a different construct with its own refusal, and it
    // is what needs a replica protocol. What this needed was one import (`dom_on_message`) and the
    // LGX end of a socket that already existed.
    //
    // ★ AND A TEST CAUSES ONE WITH THE DRIVER THAT ALREADY EXISTS. A subscription registers as a
    // page-level listener for `message:<topic>`, so `Raise … on "body" with …` delivers it —
    // `event_round_trip_lock`'s rule is met without inventing a verb.
    (
        "Message",
        "When a message arrives on \"news\" (body: Text):\n\x20       Set draft to body.",
        Tier::Lowered,
        "When a message arrives on …:",
    ),
    // ★ ONE READ IMPORT (`dom_event_file_name`). A browser raises `drop` and the page must dig the
    // file out of the transfer; what reaches the handler on every tier is the SEMANTIC event,
    // `filedropped`, with the name it carried. The driver learned to carry a value for it —
    // `Raise "…" on "…" with "…".` — through the dispatcher's existing value slot.
    (
        "FileDropped",
        "When a file is dropped on \"#b\" (name: Text):\n\x20       Set draft to name.",
        Tier::Lowered,
        "When a file is dropped on …:",
    ),
];

/// A STATEFUL page whose one handler waits for `trigger`.
///
/// Stateful on purpose: the gate this file measures only applies to a component with actions, and
/// a stateless fixture would report every row as `Lowered` while asserting nothing.
fn page_with(trigger: &str) -> String {
    format!(
        // ★ THE SCAFFOLD CARRIES A STORE so a row can watch one. `When Session's name changes:`
        // is the same `Trigger` variant as `When n changes:` and needs a store to name; the other
        // rows are unaffected by a store they never mention.
        "## Store Session\n\
         State:\n\
        \x20   Let name be \"guest\".\n\
         Actions:\n\
        \x20   To login (who: Text):\n\
        \x20       Set name to who.\n\n\
         ## Component App\n\
         Use Session.\n\
         State:\n\
        \x20   Let n be 0.\n\
        \x20   Let draft be \"\".\n\
         Actions:\n\
        \x20   To go:\n\
        \x20       Set n to n + 1.\n\
        \x20   {trigger}\n\
         View:\n\
        \x20   <div><p id=\"p\">{{n}}{{draft}}</p><button id=\"b\" on-click={{go}}>go</button></div>\n\n\
         ## Main\n\
         Mount App.\n"
    )
}

#[test]
fn the_fixture_itself_reaches_the_browser() {
    // ★ THE CONTROL. Without it a malformed fixture reports every trigger as a tier gap — which is
    // precisely what `action_verb_tier_lock`'s first draft did, seventeen times, over one blank
    // line. The page must compile with a trigger that is known to be `Lowered`.
    let src = page_with("When the page loads:\n\x20       Set n to n + 1.");
    if let Err(e) = compile_to_wasm(&src) {
        panic!("the control page must reach the browser, or no row here means anything.\nERROR: {e:?}\n\n{src}");
    }
}

#[test]
fn every_trigger_reaches_the_browser_or_names_its_own_sentence() {
    let mut wrong: Vec<String> = Vec::new();
    for (name, sentence, tier, form) in TRIGGERS {
        let src = page_with(sentence);
        match (compile_to_wasm(&src), tier) {
            (Ok(_), Tier::Lowered) => {}
            (Err(e), Tier::RefusedByName) => {
                // A refusal is only useful if the author can find the line it is about.
                let msg = format!("{e:?}");
                if !msg.contains(form) {
                    let opener: &str = sentence.split(':').next().unwrap_or(sentence).trim();
                    wrong.push(format!(
                        "{name} refuses without quoting its form — the author wrote `{opener}`, \
                         the refusal should carry `{form}`, and the message says: {msg}"
                    ));
                }
            }
            (Ok(_), Tier::RefusedByName) => wrong.push(format!(
                "{name} is catalogued as refusing and it COMPILED. If it now reaches the browser \
                 that is good news — move its row to `Lowered` so the catalogue says so."
            )),
            (Err(e), Tier::Lowered) => wrong.push(format!(
                "{name} is catalogued as reaching the browser and it does not: {e:?}"
            )),
        }
    }
    assert!(
        wrong.is_empty(),
        "trigger tiers disagree with the catalogue:\n  {}",
        wrong.join("\n  ")
    );
}

#[test]
fn the_catalogue_covers_every_trigger() {
    // ★ THE EXHAUSTIVENESS CHECK, and the reason this is a catalogue rather than a list of the
    // triggers somebody happened to think of. A new `Trigger` variant does not get to be silently
    // absent: it must be classified here, which is a one-line decision at the moment it is cheap.
    let ast = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../logicaffeine_language/src/ast/component.rs"),
    )
    .expect("the component AST");
    let body = ast
        .split("pub enum Trigger<'a> {")
        .nth(1)
        .and_then(|s| s.split("\n}").next())
        .expect("Trigger is declared there");
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

    assert!(
        declared.len() >= 17,
        "the trigger scan found only {declared:?} — it has stopped matching rather than the \
         language having stopped declaring triggers"
    );
    // ★ A ROW NAMES A VARIANT, OPTIONALLY WITH A BEHAVIOUR AFTER A `/`.
    //
    // The check used to compare row names to variant names one-for-one, and that IS what created
    // the blind spot it exists to prevent: `FieldChanges { store: Option<Symbol>, field }` carries
    // TWO sentences — `When n changes:` and `When Session's name changes:` — with two different
    // tier answers, and a 1:1 mapping cannot express the second. Worse, it REJECTED the attempt:
    // the extra row came back as "names triggers that no longer exist", so the only way to satisfy
    // the census was to leave the store form out of it. A rule that forbids recording a gap will
    // always report no gaps.
    //
    // Now `FieldChanges/store` is a row about the `FieldChanges` variant, and coverage is measured
    // against the part before the `/`. Exhaustiveness over variants is unchanged and strict; what
    // is new is that a variant whose FIELD decides the answer can say so.
    let variant_of = |row: &str| row.split('/').next().unwrap_or(row).to_string();
    let known: Vec<String> = TRIGGERS.iter().map(|(n, ..)| variant_of(n)).collect();
    let missing: Vec<&String> = declared.iter().filter(|d| !known.contains(d)).collect();
    assert!(
        missing.is_empty(),
        "these `Trigger` variants are in no catalogue row: {missing:?}\n\
         Add each with `Lowered` if it compiles for the browser, or `RefusedByName` if it does not \
         yet and says so in words naming the author's sentence. Silently absent is not one of the \
         options — that is how a `useEffect` comes to work only on the tier you test on."
    );
    // The same rule in reverse: a row is stale when the VARIANT it names is gone.
    let stale: Vec<&String> = known.iter().filter(|k| !declared.contains(k)).collect();
    assert!(
        stale.is_empty(),
        "the catalogue names triggers that no longer exist: {stale:?}"
    );
}

#[test]
fn the_partition_is_a_partition() {
    // Every row is one side or the other, and both sides are occupied — a catalogue that had
    // drifted to all-refused would still "pass" every other test in this file while describing a
    // framework where no effect works at all.
    let lowered = TRIGGERS
        .iter()
        .filter(|(_, _, t, _)| *t == Tier::Lowered)
        .count();
    let refused = TRIGGERS
        .iter()
        .filter(|(_, _, t, _)| *t == Tier::RefusedByName)
        .count();
    assert_eq!(lowered + refused, TRIGGERS.len(), "every row is classified");
    assert!(lowered > 0, "some trigger must reach the browser");
    // ★ SAYING SO, AS INSTRUCTED. This line used to read `refused > 0` with the note "if nothing
    // is refused any more, delete this half and say so" — written by someone who expected the day
    // to come and left the instruction for whoever arrived. It has: every trigger the language
    // declares now reaches a browser, `Message` last, once it turned out the relay it was waiting
    // for had been built all along.
    //
    // The half is not deleted, it is INVERTED: the catalogue still classifies every row, and a new
    // refusal is still a one-line decision — but the expected count is now zero, so a trigger
    // QUIETLY regressing to refused fails here instead of being absorbed by a `> 0`.
    assert_eq!(
        refused, 0,
        "{refused} trigger(s) no longer reach the browser. Every one did; if that is a deliberate \
         retreat, say why here, and if it is not, it is a regression the catalogue just caught."
    );
}
