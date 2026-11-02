//! ★ LOCK — a refused trigger is named, in the words the author wrote. ★
//!
//! A page whose `Actions:` waits for something the browser tier cannot deliver is refused WHOLE —
//! which is right, because rendering it with one sentence silently inert is the worse outcome. But
//! the refusal used to say only:
//!
//!     "one of its `Actions:` sentences waits for something this tier cannot deliver yet"
//!
//! ★ TRUE, AND USELESS. A page with six handlers gave the author no way to tell WHICH one, and the
//! sentence they need to find is the sentence they wrote. It is the fifth anonymous refusal this
//! campaign has had to fix — the same shape as the VM naming a fixed guess for every `Mount`
//! refusal, the AOT backend saying "unsupported expression" for a page query, seventeen action
//! verbs behind one catch-all, and a bare arity error naming no function.
//!
//! ★ THE FORM, NOT THE VALUES, and that distinction is the design. `Trigger::shape` already renders
//! a trigger — for snapshot tests — and it prints what the AST holds: `every 60000000000ms`, which
//! is true and is not what anybody typed. `Trigger::form` names the SENTENCE SHAPE (`Every … seconds:`)
//! because a refusal is read by the person who wrote the line and needs to find it. Two renderings
//! for two audiences, kept adjacent in one impl so neither changes unseen.
//!
//! What is NOT asserted here: which triggers reach the browser. That set grows as effects land, and
//! a lock pinning it would go stale the way four fixtures did in one day. What must hold is that
//! whatever is refused, is refused BY NAME.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

/// A page whose only `Actions:` sentence is `trigger`.
fn page(trigger: &str) -> String {
    format!(
        "## Component App\n\
         \n\
         State:\n\
        \x20   Let n be 0.\n\
         \n\
         Actions:\n\
        \x20   {trigger}\n\
        \x20       Set n to n + 1.\n\
         \n\
         View:\n\
        \x20   <p id=\"n\">{{n}}</p>\n\
         \n\
         ## Main\n\
         Mount App.\n"
    )
}

/// A TIER GAP: the reference tier delivers it, the browser tier does not yet. The refusal is
/// therefore about the TIER, and "it works on the tree-walker" is a true and useful thing to say.
const TIER_GAP: &[(&str, &str, &str)] = &[
    // ★ `When the route changes:` LEFT THIS LIST — the gap it recorded is closed. `onRoute` was
    // already a builtin with a `runtime.js` popstate registration, so the browser half had existed
    // all along and only the LGX trigger was unwired. It is now `Lowered` in `trigger_tier_lock`,
    // and a trigger cannot be in both places: this file is where a tier gap is ADMITTED, so
    // leaving it here would have the language apologise for something it does.
    //
    // ★ `When this component disappears:` LEFT IT TOO, the same way: the page's retained instance
    // maps already knew what was on screen, so the unmount set is a diff rather than a question
    // for the host. So did `Every … seconds:` and `After … seconds:`, once the page got ONE clock
    // and the arithmetic moved into compiled LOGOS.
    //
    // ★ AND THEN THERE WERE NONE. `When an action fails …:` was the last row, and it closed the
    // same way the others did: the missing piece was never the TRAP, it was that nothing knew
    // whose turn had trapped. The module writes that down now, so the host's only job is to report.
    //
    // An EMPTY tier-gap list is the good outcome and the tests below say so rather than asserting
    // over nothing: every sentence the reference tier delivers, the browser delivers too.
];

/// ★ NOT A TIER GAP — NOTHING delivers these yet, and the distinction was worth finding.
///
/// The reference tier refuses them AT MOUNT, by name, saying what they need ("`App` waits for the
/// window resizing, which needs a real window") — that is the honesty rule the event vocabulary
/// already follows, because a sentence sitting in a page doing nothing is the worst outcome.
///
/// They are separated from `TIER_GAP` because the two deserve different words. My first version of
/// this file put all seven in one list and asserted every one still ran on the reference tier; three
/// did not, which is how the distinction surfaced. Conflating them would have locked in a message
/// that tells the author of `When the window resizes:` that their page "works on the tree-walker" —
/// misdirection, and they would go looking for a tier to run it on.
/// ★ DERIVED, NOT CURATED — see `the_partition_is_the_one_the_compiler_actually_uses`. The variant
/// name is the first column so this list can be held against `Trigger::undeliverable`, the table
/// both tiers actually consult. It listed SIX variants while this file described three; the three
/// it did not describe were exactly the ones nobody had thought about, which is what an
/// unchecked catalogue is for.
/// ★ EMPTY, AND THAT IS THE ACHIEVEMENT. `Message` was the last row here — the one sentence NO
/// tier delivered — and the test below told us what to do when it landed: "move it out of
/// `NO_TIER_YET` rather than keeping the apology."
///
/// The list stays because the next sentence that outruns its tiers belongs in it, and because the
/// checks below are what make an empty list MEAN something: they hold `undeliverable` and this
/// list to each other, so neither can quietly grow without the other.
const NO_TIER_YET: &[(&str, &str)] = &[];

#[test]
fn a_refused_trigger_is_named_by_the_sentence_the_author_wrote() {
    // ★ EMPTY TODAY, SO THIS RUNS ZERO TIMES — and saying so is the difference between a test that
    // is satisfied and a test that is asleep. Every trigger reaches the browser now, so there is
    // no refusal whose wording to check; when a row returns, this loop is live again and the
    // assertion below stops holding, which is the reminder to re-read this note.
    assert!(
        TIER_GAP.is_empty(),
        "`TIER_GAP` has rows again — this check is doing real work, so update this note"
    );
    for (_, sentence, form) in TIER_GAP {
        let src = page(sentence);
        let err = vm_outcome(&src)
            .error
            .unwrap_or_else(|| panic!("`{sentence}` must be REFUSED, not silently run:\n{src}"));
        assert!(
            err.contains(form),
            "the refusal must name the sentence — expected `{form}` in:\n{err}"
        );
    }
}

#[test]
fn the_refusal_still_says_it_is_a_tier_limit_and_not_a_mistake() {
    // ★ NAMING THE SENTENCE MUST NOT COST THE OTHER HALF. "Your `Every … seconds:` is wrong" would
    // send the author to fix a sentence that is perfectly correct — it runs on the reference tier.
    // The message has to say WHICH line AND that the line is fine and the tier is not.
    //
    // ★★ AND THERE IS NOTHING LEFT TO SAY IT ABOUT. Every trigger this file once recorded as a
    // TIER GAP — `Every … seconds:`, `After … seconds:`, `When the route changes:`,
    // `When this component disappears:`, `When an action fails …:` — now reaches the browser, so
    // `TIER_GAP` is EMPTY and there is no refusal of that shape to read. Each of those closures
    // turned this test red in turn, which is exactly how a test written against a gap should
    // behave, and the last one leaves it with no subject.
    //
    // Asserting over an empty list would be a test that checks nothing while looking like it
    // checks something, so it asserts the ACHIEVEMENT instead: every trigger the reference tier
    // delivers, the browser delivers too. The wording rule it used to guard is not lost — it lives
    // in `a_refused_trigger_is_named_by_the_sentence_the_author_wrote` above, which reads the same
    // list — and the moment a row returns, that rule has a subject again.
    assert!(
        TIER_GAP.is_empty(),
        "a tier gap is back: {TIER_GAP:?}. Restore the wording check — a refusal that names the \
         sentence must also say the sentence is FINE and the tier is not, or its author goes \
         looking for a mistake they did not make."
    );
}

#[test]
fn every_refused_page_still_runs_on_the_reference_tier() {
    // ★ THE CLAIM UNDER THE REFUSAL. "This tier cannot deliver it yet" is only honest if another
    // one can — otherwise the sentence is broken everywhere and the message is misdirection. This
    // is what makes each row above a TIER gap rather than an unimplemented feature.
    //
    // ★ AND THERE ARE NO ROWS, so this too runs zero times. Found by sweeping the LGX locks for
    // tests that CANNOT FAIL: three catalogues had emptied as the campaign closed their subjects,
    // and only two of them had been given this guard — this file had one loop protected and two
    // asleep. A vacuity sweep is worth running whenever a gap closes, because the tests that go
    // quiet are exactly the ones nobody looks at again.
    assert!(
        TIER_GAP.is_empty(),
        "`TIER_GAP` has rows again — this check is doing real work, so update this note"
    );
    for (_, sentence, _) in TIER_GAP {
        let src = page(sentence);
        assert_eq!(
            tw_outcome(&src).error,
            None,
            "`{sentence}` must run on the reference tier, or the refusal misdescribes it:\n{src}"
        );
    }
}

#[test]
fn a_trigger_no_tier_delivers_is_refused_by_the_reference_tier_too() {
    // ★ AND IT NAMES WHAT IT NEEDS. These are not waiting on a lowering; they are waiting on a real
    // window and a real network. The reference tier says so at MOUNT rather than accepting a
    // sentence it will never fire — the rule `event_vocabulary_lock` established, checked here for
    // the three that sit outside the tier story so nobody later files them as a browser-tier gap.
    // ★ VACUOUS TODAY, ON PURPOSE. Nothing is waiting on a tier any more, so this loop runs zero
    // times — and the assertion below is what stops that from being mistaken for a passing check.
    // A test that silently does nothing is the failure this campaign kept finding; one that says
    // "there is nothing to check, and here is the fact that makes that true" is evidence.
    assert!(
        NO_TIER_YET.is_empty(),
        "`NO_TIER_YET` has rows again, so this loop is doing real work — good, but say so here"
    );
    for (_, sentence) in NO_TIER_YET {
        let src = page(sentence);
        let err = tw_outcome(&src)
            .error
            .unwrap_or_else(|| panic!("`{sentence}` must be refused, not silently inert:\n{src}"));
        assert!(
            err.contains("needs") || err.contains("waits for"),
            "the refusal must say what the sentence needs: {err}"
        );
    }
}

#[test]
fn a_trigger_the_tier_does_deliver_is_not_refused() {
    // The control. Without it every assertion above is satisfied by a tier that refuses everything,
    // and the lock would pass against a page that never renders at all.
    for sentence in ["When the page loads:", "When this component appears:"] {
        let src = page(sentence);
        logicaffeine_compile::semantics::dom::reset();
        assert_eq!(
            vm_outcome(&src).error,
            None,
            "`{sentence}` reaches this tier and must not be refused:\n{src}"
        );
    }
}

/// ★ THE PARTITION IS THE COMPILER'S, NOT THIS FILE'S.
///
/// `TIER_GAP` and `NO_TIER_YET` say two different things to an author — "your sentence is fine, run
/// it on another tier" versus "nothing runs this yet, and here is what it needs" — and telling a
/// `When the window resizes:` author the first would send them hunting for a tier that does not
/// exist. Which list a trigger belongs in is decided by ONE table, `Trigger::undeliverable`, which
/// is what the reference tier's mount refusal and the bytecode tier's component refusal both read.
///
/// So this test reads that table out of the source and holds both lists to it. It is the same
/// argument `trigger_tier_lock` makes about itself: a catalogue nothing checks is a catalogue that
/// rots. This one had rotted already — `undeliverable` named six variants and this file described
/// three, so half the refusals it exists to keep honest were outside it. And when a trigger LEAVES
/// the table (`FetchFails` just did, and `RouteChanges` before it), the stale row goes red here
/// rather than leaving the language apologising for something it now does.
#[test]
fn the_partition_is_the_one_the_compiler_actually_uses() {
    let ast = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../logicaffeine_language/src/ast/component.rs"),
    )
    .expect("the Trigger AST must be readable — it is the source of this partition");
    let body = {
        let start = ast
            .find("pub fn undeliverable")
            .expect("`Trigger::undeliverable` must exist");
        let rest = &ast[start..];
        let end = rest.find("\n    }").expect("its body must close");
        &rest[..end]
    };
    let mut named: Vec<&str> = body
        .match_indices("Trigger::")
        .map(|(i, _)| {
            let tail = &body[i + "Trigger::".len()..];
            let n = tail
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(tail.len());
            &tail[..n]
        })
        .collect();
    named.sort_unstable();
    named.dedup();
    // ★ THE INVARIANT IS AGREEMENT, NOT NON-EMPTINESS. This used to demand the table name at
    // least one variant, which was true right up until the last one was delivered — a fixture
    // built on a gap, failing the moment the gap closed. What actually matters is that
    // `Trigger::undeliverable` and `NO_TIER_YET` describe the SAME set, so a sentence cannot be
    // refused by the compiler while this file believes it works, or the reverse. Two empty sets
    // agree; so do two matching non-empty ones.
    assert_eq!(
        named.len(),
        NO_TIER_YET.len(),
        "`Trigger::undeliverable` names {named:?} and `NO_TIER_YET` has {} rows — the compiler and \
         this catalogue disagree about which sentences no tier delivers",
        NO_TIER_YET.len()
    );

    for variant in &named {
        assert!(
            NO_TIER_YET.iter().any(|(n, _)| n == variant),
            "`Trigger::{variant}` is in `undeliverable` — NO tier delivers it — so it needs a \
             `NO_TIER_YET` row here saying what it waits for. Without one its refusal is never \
             checked, which is how three of these sat outside this file entirely."
        );
        assert!(
            !TIER_GAP.iter().any(|(n, _, _)| n == variant),
            "`Trigger::{variant}` cannot be a TIER GAP: `undeliverable` says no tier delivers it, \
             so telling its author the sentence 'works on the tree-walker' is misdirection."
        );
    }
    for (variant, sentence) in NO_TIER_YET {
        assert!(
            named.contains(variant),
            "`Trigger::{variant}` is no longer in `undeliverable`, so some tier now delivers \
             `{sentence}` — move it out of `NO_TIER_YET` rather than keeping the apology."
        );
    }
}
