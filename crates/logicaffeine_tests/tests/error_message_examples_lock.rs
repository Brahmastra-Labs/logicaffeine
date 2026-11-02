//! ★ LOCK — every LGX construct an error message TEACHES must actually parse.
//!
//! An error message that carries an example is doing the most important teaching the language
//! does: it reaches an author at the exact moment they are stuck, and they will copy what it says.
//! So an example that does not parse is worse than no example — it spends the author's trust and
//! sends them somewhere that fails again, with a second error that does not mention the first.
//!
//! ★ THIS EXISTS BECAUSE THREE MESSAGES TAUGHT `count of todos`, WHICH HAS NEVER PARSED. `count`
//! is a DOM query in this language (`the count of "li"`); the sequence operator is `length of`,
//! which is what `docs/web.md` has always used. The phrase was written once and copied twice, and
//! nothing compared any of the three against the parser — the messages are strings, and a string
//! is never wrong at compile time. It was found by writing a REAL page, hitting the error, and
//! doing exactly what it said.
//!
//! Each row is a snippet lifted from a message VERBATIM, wrapped in the smallest page that gives
//! it somewhere to live. A message that changes its example must change its row, which is the
//! point: the row is the thing that checks the message is still true.

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::semantics::dom;

fn parses(what: &str, src: &str) {
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(
        out.error, None,
        "an error message teaches `{what}`, and it does not parse. An example that fails is worse \
         than none: the author copies it and gets a second error that does not mention the first.\
         \n\nThe page it was tried in:\n{src}"
    );
}

/// A `State:` field, as the "A `State:` section declares data" message spells them.
#[test]
fn the_state_section_message_teaches_forms_that_parse() {
    for (what, field) in [
        ("Let draft be \"\".", "Let draft be \"\"."),
        ("Let count: Int be 0.", "Let count: Int be 0."),
        (
            "Let remaining always be length of todos.",
            "Let todos be [\"a\"].\n\x20   Let remaining always be length of todos.",
        ),
    ] {
        parses(
            what,
            &format!(
                "## Component App\nState:\n\x20   {field}\nView:\n\x20   <p>ok</p>\n\n\
                 ## Main\nMount App.\n"
            ),
        );
    }
}

/// A HOLE, as the "a hole holds a single value" and "This `{}` is empty" messages spell them.
#[test]
fn the_hole_messages_teach_holes_that_parse() {
    for (what, state, hole) in [
        ("{name}", "Let name be \"x\".", "{name}"),
        (
            "{length of todos}",
            "Let todos be [\"a\"].",
            "{length of todos}",
        ),
    ] {
        parses(
            what,
            &format!(
                "## Component App\nState:\n\x20   {state}\nView:\n\x20   <p>{hole}</p>\n\n\
                 ## Main\nMount App.\n"
            ),
        );
    }
}

/// The DIRECTIVES the `MalformedDirective` message spells out.
#[test]
fn the_directive_message_teaches_directives_that_parse() {
    parses(
        "on-click={add} / bind={draft} / class:done={flag}",
        "## Component App\nState:\n\x20   Let draft be \"\".\n\x20   Let flag be true.\n\
         Actions:\n\x20   To add:\n\x20       Set draft to \"\".\n\
         View:\n\x20   <div class:done={flag}><input bind={draft} />\
         <button on-click={add}>go</button></div>\n\n## Main\nMount App.\n",
    );
}

/// The TAGS the `MalformedTag` message lists, in the shapes it gives them.
#[test]
fn the_tag_message_teaches_tags_that_parse() {
    parses(
        "{if} / {else:} / {for each} / {let} / {end}",
        "## Component App\nState:\n\x20   Let todos be [\"a\", \"b\"].\n\x20   Let on be true.\n\
         View:\n\x20   <ul>\n\
         \x20   {if on:}<li>yes</li>{else:}<li>no</li>{end}\n\
         \x20   {for each todo in todos:}<li>{todo}</li>{end}\n\
         \x20   {let n be 1}<li>{n}</li>\n\
         \x20   </ul>\n\n## Main\nMount App.\n",
    );
}

/// ★ EVERY TRIGGER SENTENCE A DIAGNOSTIC SPELLS OUT, in the smallest page that gives it a home.
///
/// The trigger catalogue is the framework's user-facing surface — `When "#draft" is clicked:` is
/// the second thing anyone writes — and the diagnostics that spell these out are read at exactly
/// the moment an author got one wrong. A schema (`When "<sel>" is <event>:`) cannot be checked and
/// is not listed; a CONCRETE sentence can be, and every concrete one a message offers is here.
///
/// ★ THE SCAFFOLD IS THE PART THAT DOES NOT GENERALIZE. A trigger is not a program: it needs a
/// component, a state field to touch and an action to run. That is why these rows are written by
/// hand rather than scraped out of the message strings — a scraper would have to invent the
/// scaffold, and would quietly skip whatever it could not, which is the exact failure this lock
/// exists to catch.
#[test]
fn every_concrete_trigger_a_diagnostic_offers_parses() {
    for sentence in [
        "When the page loads:",
        "When this component appears:",
        "When \"#draft\" is clicked:",
        "When \"#add\" is submitted:",
        "When \"#b\" is focused:",
        "When the user types in \"#x\":",
        "When the user presses Escape:",
        "When the user presses Escape in \"#draft\":",
        "When an action fails (e: Text):",
        "When todos change:",
    ] {
        parses(
            sentence,
            &format!(
                "## Component App\nState:\n\x20   Let todos be [\"a\"].\n\x20   Let n be 0.\n\
                 Actions:\n\x20   {sentence}\n\x20       Set n to n + 1.\n\
                 View:\n\x20   <div id=\"x\"><input id=\"draft\" /><button id=\"b\">b</button>\
                 <form id=\"add\"></form><p>{{n}}</p></div>\n\n## Main\nMount App.\n"
            ),
        );
    }
}

/// The `State:` forms a diagnostic offers beyond the three in the section message.
#[test]
fn the_mutable_state_form_a_diagnostic_offers_parses() {
    parses(
        "Let mutable n be 5.",
        "## Main\nLet mutable n be 5.\nShow n.\n",
    );
}

/// A `## Store` and the two sentences that reach it, as the store diagnostics spell them.
#[test]
fn the_store_sentences_a_diagnostic_offers_parse() {
    parses(
        "Use <Store>. / When Session's name changes:",
        "## Store Session\nState:\n\x20   Let name be \"\".\n\
         Actions:\n\x20   To rename (to: Text):\n\x20       Set name to to.\n\n\
         ## Component App\n\
         Use Session.\n\
         State:\n\x20   Let seen be 0.\n\
         Actions:\n\x20   When Session's name changes:\n\x20       Set seen to seen + 1.\n\
         \x20   To go:\n\x20       Call Session's rename with \"x\".\n\
         View:\n\x20   <button on-click={go}>{seen}</button>\n\n## Main\nMount App.\n",
    );
}

/// The event qualifiers the event-modifier refusals teach: one key per handler on a key event,
/// `.prevent` on a submit, a single character as a key.
#[test]
fn the_event_qualifier_messages_teach_handlers_that_parse() {
    parses(
        "on-keydown.enter={…} / on-keydown.escape={…} / on-submit.prevent / on-keyup.a",
        "## Component App\nState:\n\x20   Let n be 0.\nActions:\n\x20   To add:\n\x20       Set n to n + 1.\n\
         View:\n\x20   <form on-submit.prevent={add}>\
         <input aria-label=\"d\" on-keydown.enter={add} on-keyup.a={add}>\
         <input aria-label=\"e\" on-keydown.escape={add} on-keypress.space={add}></form>\n\n## Main\nMount App.\n",
    );
}

/// The journal sentence the `Mount` refusal shows for a capitalised name at a file path.
#[test]
fn the_mount_message_teaches_a_journal_mount_that_parses() {
    parses(
        "Mount ledger at \"data/ledger.journal\".",
        "## Main\nMount ledger at \"data/ledger.journal\".\n",
    );
}

/// What the placement refusals teach: every declared prop passed, a callback passed by the
/// action's name, a callback prop declared `fn(…) -> Unit`.
#[test]
fn the_placement_messages_teach_placements_that_parse() {
    parses(
        "done={…} / choose={actionName} / (choose: fn(Text) -> Unit)",
        "## Component Pick (label: Text) and (done: Bool) and (choose: fn(Text) -> Unit)\n\
         View:\n\x20   <button on-click={choose(label)}>{label}</button>\n\n\
         ## Component App\nState:\n\x20   Let picked be \"\".\nActions:\n\x20   To take (t: Text):\n\x20       Set picked to t.\n\
         View:\n\x20   <Pick label=\"a\" done={true} choose={take}/>\n\n## Main\nMount App.\n",
    );
}

/// What the resolution refusals teach: a store read after `Use`, a state field a `bind` writes.
#[test]
fn the_resolution_messages_teach_forms_that_parse() {
    parses(
        "Use Session. / bind={draft} over a `State:` field",
        "## Store Session\nState:\n\x20   Let user be \"ada\".\n\n\
         ## Component App\nUse Session.\nState:\n\x20   Let draft be \"\".\n\
         View:\n\x20   <p>{Session's user}</p><input aria-label=\"d\" bind={draft}>\n\n## Main\nMount App.\n",
    );
}

/// The canonical spellings the refusals of `count of xs`, `{elif}` and `{otherwise}` give.
#[test]
fn the_wave_four_messages_teach_forms_that_parse() {
    parses(
        "length of xs / the count of \".row\" / {else if c:} / {else:} / {end}",
        "## Component App\nState:\n\x20   Let xs be [\"a\"].\n\x20   Let c be true.\n\
         View:\n\x20   <ul>{for each x in xs:}<li class=\"row\">{x}</li>{end}</ul>\
         <p>{length of xs}</p>\
         {if length of xs is 0:}<p>none</p>{else if c:}<p>some</p>{else:}<p>off</p>{end}\n\n\
         ## Main\nMount App.\nShow the count of \".row\".\n",
    );
}
