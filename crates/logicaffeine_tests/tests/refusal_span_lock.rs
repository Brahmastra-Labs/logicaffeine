//! ★ LOCK — a browser-tier refusal points at the line that caused it. ★
//!
//! `NotLowerable` carried a message and NOTHING ELSE. Every refusal in `lgx_lower` walks the view
//! tree, and every one of them destructured the node with `..` — so a page refused because of ONE
//! element, or ONE handler, reported at `Span::default()`, which renders as the top of the file.
//!
//! ★ AND THE MESSAGES HAD ALREADY BEEN FIXED ONCE FOR EXACTLY THIS. `Trigger::form` exists because
//! "one of its `Actions:` sentences waits for something this tier cannot deliver yet" gave the
//! author of a page with six handlers no way to tell WHICH one. That fix named the construct. It
//! could not say where the construct was, because there was nowhere to put the answer — so half
//! the question stayed open, and a page with two `Every … seconds:` handlers is still a search.
//!
//! Two halves, and the first is structural rather than tested here: `NotLowerable::at` takes the
//! span as a REQUIRED argument, so a new refusal does not compile until it says where it happened.
//! The previous shape made "no span" the path of least resistance and all twenty-one sites took it.
//!
//! What this file adds is the half a signature cannot express: that the span is a REAL location and
//! not a placeholder someone passed to satisfy the compiler. Each case asserts the source text AT
//! the span, so `Span::default()`, an empty span, or the span of the wrong construct all go red.

use logicaffeine_base::{Arena, Interner, Symbol};
use logicaffeine_language::analysis::{DiscoveryPass, TypeRegistry};
use logicaffeine_language::arena_ctx::AstContext;
use logicaffeine_language::ast::component::ComponentDef;
use logicaffeine_language::ast::{Expr, LogicExpr, NounPhrase, Stmt, Term, ThematicRole, TypeExpr};
use logicaffeine_language::drs::WorldState;
use logicaffeine_language::parser::lgx_lower::lowerable;
use logicaffeine_language::{Lexer, Parser};

/// Parse `src` and ask the browser tier about its first component: `Ok` if it lowers, otherwise
/// the refusal's message and the SOURCE TEXT its span covers.
fn refusal_of(src: &str) -> Result<(), (String, String)> {
    let interner: &'static mut Interner = Box::leak(Box::new(Interner::new()));
    let world_state: &'static mut WorldState = Box::leak(Box::new(WorldState::new()));
    let expr_arena: &'static Arena<LogicExpr> = Box::leak(Box::new(Arena::new()));
    let term_arena: &'static Arena<Term> = Box::leak(Box::new(Arena::new()));
    let np_arena: &'static Arena<NounPhrase> = Box::leak(Box::new(Arena::new()));
    let sym_arena: &'static Arena<Symbol> = Box::leak(Box::new(Arena::new()));
    let role_arena: &'static Arena<(ThematicRole, Term)> = Box::leak(Box::new(Arena::new()));
    let pp_arena: &'static Arena<&'static LogicExpr> = Box::leak(Box::new(Arena::new()));
    let stmt_arena: &'static Arena<Stmt> = Box::leak(Box::new(Arena::new()));
    let iexpr_arena: &'static Arena<Expr> = Box::leak(Box::new(Arena::new()));
    let type_arena: &'static Arena<TypeExpr> = Box::leak(Box::new(Arena::new()));
    let ctx = AstContext::with_types(
        expr_arena,
        term_arena,
        np_arena,
        sym_arena,
        role_arena,
        pp_arena,
        stmt_arena,
        iexpr_arena,
        type_arena,
    );
    let mut lexer = Lexer::new(src, interner);
    let tokens = lexer.tokenize();
    let registry: TypeRegistry = DiscoveryPass::new(&tokens, interner).run_full().types;
    let types = registry.clone();
    let mut parser = Parser::new(tokens, world_state, interner, ctx, registry);
    let stmts = parser
        .parse_program()
        .unwrap_or_else(|e| panic!("must parse: {e:?}\n{src}"));
    drop(parser);

    let defs: Vec<ComponentDef<'static>> = stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::ComponentDef(d) => Some((**d).clone()),
            _ => None,
        })
        .collect();
    let first = defs.first().expect("the fixture must declare a component");
    match lowerable(first, &defs, Some(&types), interner) {
        Ok(()) => Ok(()),
        Err(why) => {
            let at = src
                .get(why.span.start..why.span.end)
                .unwrap_or("")
                .to_string();
            Err((why.message, at))
        }
    }
}

/// The view every fixture renders unless it is the view itself under test: one bound field and one
/// handler that lowers, so a page is only ever refused for the construct being measured.
const CONTROL_VIEW: &str =
    "<div><p id=\"p\">{n}</p><button id=\"b\" on-click={go}>go</button></div>";

/// A stateful page with `handlers` in `Actions:` and `view` in `View:` — the shape
/// `trigger_tier_lock` uses, because the tier gate this file measures only applies to a component
/// with actions.
fn page(handlers: &str, view: &str) -> String {
    format!(
        "## Component App\n\
         State:\n\
        \x20   Let n be 0.\n\
        \x20   Let draft be \"\".\n\
        \x20   Let kind be \"text\".\n\
         Actions:\n\
        \x20   To go:\n\
        \x20       Set n to n + 1.\n\
        \x20   {handlers}\n\
         View:\n\
        \x20   {view}\n\n\
         ## Main\n\
         Mount App.\n"
    )
}

/// A page whose handlers are `handlers` and whose view is the control.
fn page_with(handlers: &str) -> String {
    page(handlers, CONTROL_VIEW)
}

#[test]
fn the_control_page_lowers() {
    // ★ WITHOUT THIS EVERY ROW BELOW IS FREE. A malformed fixture refuses for a reason that has
    // nothing to do with what is being measured, and the span assertions pass on the wrong error.
    let src = page_with("When the page loads:\n\x20       Set n to n + 1.");
    assert_eq!(
        refusal_of(&src),
        Ok(()),
        "the control page must reach the browser, or nothing else here means anything:\n{src}"
    );
}

#[test]
fn a_refused_construct_reports_at_its_own_source() {
    // ★ THE CASE THAT MOTIVATES ALL OF THIS. TWO constructs of a kind, and only the second cannot
    // lower. A span that is `Span::default()`, or the component's, or the first one's, all point
    // the author somewhere they would have to read past to find the problem.
    //
    // The refused construct is drawn from STILL_REFUSED rather than written in, because this case
    // decayed once already: it used `Every 60 seconds:`, the browser tier learned to lower timers,
    // and `expect_err` then panicked on an `Ok` — a red test measuring a gap that had closed.
    //
    // ★ AND IT READS THE WHOLE CATALOGUE, NOT ITS FIRST ROW. Asserting only `first()` meant three
    // of four rows could point at nothing and this still passed; the cost of checking all of them
    // is one parse each.
    assert!(!STILL_REFUSED.is_empty(), "{EMPTY_CATALOGUE}");
    let mut wrong = Vec::new();
    for g in STILL_REFUSED {
        let Gap {
            subject,
            placed,
            source,
            span_marker,
            names,
        } = g;
        let src = page_with_second(*placed, source);
        match refusal_of(&src) {
            Ok(()) => wrong.push(format!(
                "  {subject} (`{source}`): listed in STILL_REFUSED but the page LOWERED — the gap \
                 closed, so move it to NOW_LOWERED rather than leaving a row that measures nothing"
            )),
            Err((message, at)) if !at.contains(span_marker) => wrong.push(format!(
                "  {subject} (`{source}`): refused with {message:?} but its span covers {at:?}, \
                 which does not contain {span_marker:?} — it is the SECOND of two, so pointing \
                 anywhere else (the top of the file, the component, the one that was fine) leaves \
                 the author searching"
            )),
            // ★ THE SPAN ALONE IS NOT ENOUGH. The fixture CONTAINS the refused construct's text, so
            // a different refusal whose span happened to cover that element would satisfy the check
            // above while this row quietly measured some other gap.
            Err((message, _)) if !message.contains(names) => wrong.push(format!(
                "  {subject} (`{source}`): refused, and at the right place, but for another \
                 reason — {message:?} does not carry {names:?}, so this row is no longer measuring \
                 the gap it was written for"
            )),
            Err(_) => {}
        }
    }
    assert!(
        wrong.is_empty(),
        "a browser-tier refusal must point AT the construct that caused it:\n{}",
        wrong.join("\n")
    );
}

#[test]
fn a_component_with_no_view_reports_at_the_component() {
    let src = "## Component App\n\
               State:\n\
              \x20   Let n be 0.\n\
               Actions:\n\
              \x20   To go:\n\
              \x20       Set n to n + 1.\n\n\
               ## Main\n\
               Mount App.\n";
    let (message, at) = refusal_of(src).expect_err("a component with no `View:` cannot render");
    assert!(
        at.contains("App"),
        "a refusal about the component as a whole must point at the component. \
         message: {message:?}\nspan covers: {at:?}\n{src}"
    );
}

/// Where a still-refused construct is WRITTEN, because they are no longer all `Actions:` sentences.
///
/// ★ THE TRIGGER CATALOGUE CLOSED COMPLETELY, so a fixture that can only place a handler can no
/// longer reach a single remaining gap. `Trigger::undeliverable` answers `None` for every variant
/// it has — `Message` was the last row and its LGX end landed — and a table keyed to that enum now
/// describes an empty set. The gaps that remain are in the VIEW, so the partition is over
/// browser-tier gaps wherever they live, rather than over one syntactic category that ran out.
#[derive(Clone, Copy, Debug)]
enum Placed {
    /// An `Actions:` sentence the page waits on: `When a file is dropped on "#zone":`.
    ///
    /// ★ NO ROW USES THIS TODAY, AND THAT IS A MEASUREMENT, NOT AN OVERSIGHT. The handler gate in
    /// `lgx_lower` is an allowlist of the triggers that lower, and MEASURED against `ast::component`
    /// it now names all seventeen `Trigger` variants — so the refusal beneath it is unreachable and
    /// the handler category is empty. The variant stays because that is precisely the state this
    /// file exists to watch: the eighteenth trigger, added without being allowlisted, is refused
    /// there and its row belongs here.
    #[allow(dead_code)]
    Handler,
    /// An `on-…` attribute written on an element in `View:`: `on-click.stop={go}`.
    Attr,
    /// A whole element written in `View:`, refused for how its attributes combine:
    /// `<input type={kind} bind={draft}>`.
    Element,
}

/// Browser-tier gaps that are still gaps, as
/// `(subject, where it is written, the source fragment, a distinctive fragment the span must cover)`.
///
/// ★ THIS FILE HAS NOW DECAYED THREE TIMES, AND THE LAST TIME EMPTIED THE CATEGORY. First
/// `When the route changes:` started lowering; then timers, component-disappears, window-resize and
/// goes-offline all landed together; then `When a message arrives on …:` — the row this table was
/// reduced to — landed as well, and `Trigger::undeliverable` became the empty match it is today.
/// A catalogue of examples decays silently as the gaps close, so this is a PARTITION instead: what
/// is still refused must refuse WITH A REAL SPAN, and what now lowers must KEEP lowering. The
/// second half is not bookkeeping — it is how a closed gap re-opening is caught, which a list of
/// refusals alone can never see.
///
/// ★ THE BEHAVIOURAL EVENT MODIFIERS CLOSED TOO, so the partition moved to a refusal that is
/// permanent by nature: a `bind` on an element whose `type` is computed. Whether it writes `value`
/// or a checkbox's `checked` state is undecidable until the page runs, so the lowering refuses it by
/// name rather than guess — and a guess is exactly the hazard this lock exists for, a page that
/// works on the tree-walker its author tests on and behaves differently in the browser they ship to.
struct Gap {
    /// What it is, in words, for a failure message.
    subject: &'static str,
    /// Where in a page it is written.
    placed: Placed,
    /// The source fragment that asks for it.
    source: &'static str,
    /// A distinctive fragment the refusal's SPAN must cover.
    span_marker: &'static str,
    /// A fragment the refusal's MESSAGE must carry that NO OTHER ROW'S MESSAGE DOES.
    ///
    /// ★ THE OBVIOUS CHECK IS VACUOUS HERE, which is why this is a field rather than the modifier
    /// name derived from `source`. The refusal recites the whole set — "`.stop`, `.once`,
    /// `.capture` and `.passive` shape how the event is DISPATCHED" — so asking whether the message
    /// contains `stop` is satisfied by all four messages equally, and every row would pass on every
    /// other row's refusal. The message names the one it is ABOUT in its opening clause; that is
    /// the part that distinguishes them.
    names: &'static str,
}

const STILL_REFUSED: &[Gap] = &[Gap {
    subject: "a bind on a computed input type",
    placed: Placed::Element,
    source: "<input id=\"c\" aria-label=\"c\" type={kind} bind={draft}>",
    span_marker: "type={kind}",
    names: "whose `type` is computed",
}];

/// The behavioural event modifiers — refused for the browser once, lowered now — each on a SECOND
/// handler of the control view, the position the partition measured them in.
const MODIFIERS_NOW_LOWERED: &[(&str, &str)] = &[
    ("stopping propagation", "on-click.stop"),
    ("a once-only listener", "on-click.once"),
    ("the capture phase", "on-click.capture"),
    ("a passive listener", "on-click.passive"),
];

/// A page whose SECOND construct of its kind is the refused one, the first being one that lowers.
///
/// ★ THE "SECOND OF TWO" IS THE MEASUREMENT. A span that is `Span::default()`, or the component's,
/// or the FIRST construct's, all point the author somewhere they would have to read past to find
/// the problem — and all three of those are indistinguishable from a correct span on a page that
/// contains only the refused construct.
fn page_with_second(placed: Placed, refused: &str) -> String {
    match placed {
        Placed::Handler => page_with(&format!(
            "When the page loads:\n\
            \x20       Set n to n + 1.\n\
            \x20   {refused}\n\
            \x20       Set n to n + 1."
        )),
        // The control view's own `on-click={go}` is the first handler; the refused modifier rides a
        // second button, so the span has somewhere wrong it could plausibly land.
        Placed::Attr => page(
            "When the page loads:\n\x20       Set n to n + 1.",
            &format!(
                "<div><p id=\"p\">{{n}}</p>\
                 <button id=\"b\" on-click={{go}}>go</button>\
                 <button id=\"c\" {refused}={{go}}>x</button></div>"
            ),
        ),
        // A first bound input lowers; the refused one is the second element of its kind.
        Placed::Element => page(
            "When the page loads:\n\x20       Set n to n + 1.",
            &format!(
                "<div><p id=\"p\">{{n}}</p>\
                 <input id=\"a\" aria-label=\"a\" type=\"text\" bind={{draft}}>\
                 {refused}</div>"
            ),
        ),
    }
}

/// Fixtures that USED to refuse and now reach the browser. Each is asserted to LOWER, so a
/// regression that re-refuses one is as loud as a refusal with no location.
const NOW_LOWERED: &[(&str, &str)] = &[
    ("a route change", "When the route changes:"),
    ("a timer handler", "Every 60 seconds:"),
    (
        "a component disappearing",
        "When this component disappears:",
    ),
    ("the window resizing", "When the window resizes:"),
    ("going offline", "When the app goes offline:"),
    // ★ THE TRIGGER CATALOGUE CLOSED WHILE THIS FILE WATCHED. Every one of these was
    // `RefusedByName` and is now `Lowered`, which is the shape a two-sided ratchet is for: the
    // rows above prove a gap really closed, and the rows below prove it cannot re-open quietly.
    (
        "a fetch failing",
        "When a fetch fails (url: Text) and (why: Text):",
    ),
    (
        "a key press on the page",
        "When the user presses \"Escape\":",
    ),
    (
        "scrolling to the bottom",
        "When the user scrolls to the bottom:",
    ),
    (
        "a file drop",
        "When a file is dropped on \"#zone\" (name: Text):",
    ),
    ("an action failing", "When an action fails (e: Text):"),
    ("coming back online", "When the app comes back online:"),
    ("a one-shot timer", "After 5 seconds:"),
    // ★ THE ROW THAT CLOSED THE CATEGORY. This was the last entry in `Trigger::undeliverable` and
    // the last row of `STILL_REFUSED`, held back not by a lowering but by the realtime transport —
    // which turned out to be BUILT (`logicaffeine_system::relay` already carried
    // `Subscribe`/`Publish`/`Event` between a native node and a browser tab). What was missing was
    // the LGX end. It is asserted to LOWER here for the same reason as every row above it: a gap
    // that closed is the easiest kind to re-open by accident.
    (
        "a message on a topic",
        "When a message arrives on \"t\" (m: Text):",
    ),
];

/// The `Actions:` VERBS that were refused for the browser and now lower — the verb half of the
/// same ratchet.
///
/// ★ THEY NEED THEIR OWN LIST BECAUSE THEY ARE NOT TRIGGERS. A trigger is a `When …:` sentence the
/// page waits on; these are things the page DOES, written inside an action, so the fixture puts
/// them in a body rather than in a handler position. The verb catalogue is now 21/21, and `Reload`
/// was the last — refused for a real ordering reason (a turn writes its state back at the END, so
/// clearing every instance mid-body meant the write-back re-created what it had just cleared)
/// rather than for a missing mechanism.
const VERBS_NOW_LOWERED: &[(&str, &str)] = &[
    ("focusing an element", "Focus \"#draft\"."),
    ("blurring an element", "Blur \"#draft\"."),
    ("hovering an element", "Hover \"#draft\"."),
    ("scrolling to an element", "Scroll to \"#draft\"."),
    (
        "selecting an element's text",
        "Select the text in \"#draft\".",
    ),
    (
        "writing to the clipboard",
        "Copy \"hello\" to the clipboard.",
    ),
    ("raising a notification", "Notify \"Saved.\"."),
    ("reloading the page", "Reload the page."),
];

const EMPTY_CATALOGUE: &str =
    "STILL_REFUSED is empty — every browser-tier gap has closed, so this \
     lock now measures nothing. Retire it deliberately, or add the next refused construct from \
     `lgx_lower`'s own refusals; do not leave it passing vacuously. The trigger category emptied \
     once already and the answer was to widen the partition past triggers, not to drop it.";

#[test]
fn no_refusal_lands_on_an_empty_span() {
    // ★ THE PARTITION, not a list of examples. Whatever each of these is refused FOR, none of them
    // may answer with a location that points at nothing — which is what `Span::default()` is, and
    // what every one of these used to return.
    assert!(!STILL_REFUSED.is_empty(), "{EMPTY_CATALOGUE}");
    let mut wrong = Vec::new();
    for Gap {
        subject,
        placed,
        source,
        ..
    } in STILL_REFUSED
    {
        let src = page_with_second(*placed, source);
        match refusal_of(&src) {
            Ok(()) => wrong.push(format!(
                "  {subject}: LOWERED — it reaches the browser now, so this row is measuring \
                 nothing. Move it to NOW_LOWERED."
            )),
            Err((message, at)) if at.trim().is_empty() => wrong.push(format!(
                "  {subject}: refused with {message:?} at an EMPTY span — the author is told what \
                 is wrong and pointed at the top of the file"
            )),
            Err(_) => {}
        }
    }
    assert!(
        wrong.is_empty(),
        "browser-tier refusals with no location:\n{}",
        wrong.join("\n")
    );
}

#[test]
fn a_closed_gap_stays_closed() {
    // The other half of the partition. Each of these was refused once and now lowers; asserting it
    // still does turns this file from a record of what is missing into a ratchet on what landed.
    let mut regressed = Vec::new();
    for (subject, sentence) in NOW_LOWERED {
        let src = page_with(&format!("{sentence}\n\x20       Set n to n + 1."));
        if let Err((message, at)) = refusal_of(&src) {
            regressed.push(format!(
                "  {subject} (`{sentence}`): refused again with {message:?} at {at:?}"
            ));
        }
    }
    // ★ THE VERBS GO THROUGH THE SAME GATE, IN A BODY RATHER THAN A HANDLER POSITION. A verb is
    // something the page DOES, so it is written inside an action; a trigger is a sentence the page
    // WAITS on. One ratchet, two fixtures, because the two are not interchangeable.
    for (subject, sentence) in VERBS_NOW_LOWERED {
        let src = page_with(&format!("When \"#b\" is clicked:\n\x20       {sentence}"));
        if let Err((message, at)) = refusal_of(&src) {
            regressed.push(format!(
                "  {subject} (`{sentence}`): refused again with {message:?} at {at:?}"
            ));
        }
    }
    for (subject, modifier) in MODIFIERS_NOW_LOWERED {
        let src = page_with_second(Placed::Attr, modifier);
        if let Err((message, at)) = refusal_of(&src) {
            regressed.push(format!(
                "  {subject} (`{modifier}`): refused again with {message:?} at {at:?}"
            ));
        }
    }
    assert!(
        regressed.is_empty(),
        "these reached the browser and no longer do — a closed gap re-opened:\n{}",
        regressed.join("\n")
    );
}
