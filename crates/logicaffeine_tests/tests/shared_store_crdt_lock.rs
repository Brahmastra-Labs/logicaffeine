//! ★ LOCK — a CONVERGENT field of a `shared on` store merges every page's operations; none is lost
//! to another page's concurrent write. ★
//!
//! A plain shared field is a last-writer-wins register: two pages that change it at once keep ONE
//! of the two changes. A field declared `Tally`, `ConvergentCount`, `SharedSet of T` or
//! `SharedSequence of T` keeps them all. Its author works with the plain value it converges to — an
//! `Int`, a `Set of T`, a `Seq of T` — and the page reads each action's operations off how that value
//! moved: a counter's difference is this page's increments or decrements, a set's added and removed
//! elements are tagged adds and tombstones of the adds this page had seen, a sequence's edit is
//! insert-after nodes and tombstones. The frame is the CRDT, and joining two frames is a union, so
//! pages that have seen the same frames hold the same value in whatever order the frames came.
//!
//! Every row runs on BOTH lowered engines, each tab a page of its own (`support/shared_tabs.rs`),
//! with this file playing the relay.

#[path = "support/shared_tabs.rs"]
mod shared_tabs;

use logicaffeine_compile::semantics::dom;
use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_compile::vm::session::Session;
use logicaffeine_compile::vm::{Compiler, Vm};
use shared_tabs::{items, on_each_tier, shown, Tab};

const BOARD: &str = "\
## Store Board shared on \"board\"
State:
    Let likes: Tally be 10.
    Let visits: ConvergentCount be 0.
    Let guests: SharedSet of Text be a new Set of Text.
    Let seats: SharedSet of Int be a new Set of Int.
    Let lines: SharedSequence of Text be a new Seq of Text.
Actions:
    To like:
        Increase likes by 1.
    To unlike:
        Decrease likes by 1.
    To visit:
        Increase visits by 1.
    To unvisit:
        Decrease visits by 1.
    To addAnn:
        Add \"ann\" to guests.
    To addBob:
        Add \"bob\" to guests.
    To dropAnn:
        Remove \"ann\" from guests.
    To takeSeats:
        Add 10 to seats.
        Add 9 to seats.
    To sayA:
        Push \"a\" to lines.
    To sayB:
        Push \"b\" to lines.
    To sayC:
        Push \"c\" to lines.
    To dropFirstLine:
        Set lines to items 2 through (length of lines) of lines.
    To sayBoth:
        Push \"x\" to lines.
        Push \"y\" to lines.

## Component App
Use Board.
Actions:
    To like:
        Call Board's like.
    To unlike:
        Call Board's unlike.
    To visit:
        Call Board's visit.
    To unvisit:
        Call Board's unvisit.
    To addAnn:
        Call Board's addAnn.
    To addBob:
        Call Board's addBob.
    To dropAnn:
        Call Board's dropAnn.
    To takeSeats:
        Call Board's takeSeats.
    To sayA:
        Call Board's sayA.
    To sayB:
        Call Board's sayB.
    To sayC:
        Call Board's sayC.
    To dropFirstLine:
        Call Board's dropFirstLine.
    To sayBoth:
        Call Board's sayBoth.
View:
    <p id=\"likes\">{Board's likes}</p>
    <p id=\"visits\">{Board's visits}</p>
    <ul id=\"guests\">{for each g in Board's guests:}<li>{g}</li>{end}</ul>
    <ul id=\"seats\">{for each s in Board's seats:}<li>{s}</li>{end}</ul>
    <ol id=\"lines\">{for each l in Board's lines:}<li>{l}</li>{end}</ol>
    <button id=\"like\" on-click={like}>+</button>
    <button id=\"unlike\" on-click={unlike}>-</button>
    <button id=\"visit\" on-click={visit}>v</button>
    <button id=\"unvisit\" on-click={unvisit}>u</button>
    <button id=\"ann\" on-click={addAnn}>ann</button>
    <button id=\"bob\" on-click={addBob}>bob</button>
    <button id=\"drop-ann\" on-click={dropAnn}>drop</button>
    <button id=\"seats-in\" on-click={takeSeats}>seats</button>
    <button id=\"say-a\" on-click={sayA}>a</button>
    <button id=\"say-b\" on-click={sayB}>b</button>
    <button id=\"say-c\" on-click={sayC}>c</button>
    <button id=\"drop-line\" on-click={dropFirstLine}>drop line</button>
    <button id=\"say-both\" on-click={sayBoth}>x y</button>
";

const KEYS: [&str; 5] = [
    "board.likes",
    "board.visits",
    "board.guests",
    "board.seats",
    "board.lines",
];

/// Hand every frame `from` holds to `to`, as a relay syncing a page does.
fn sync(from: &mut dyn Tab, to: &mut dyn Tab) {
    for key in KEYS {
        if let Some(frame) = from.held(key) {
            to.arrive("board", key, &frame);
        }
    }
}

/// Both pages hand each other everything they hold — the one-round exchange after which two pages
/// have seen the same frames.
fn exchange(p: &mut dyn Tab, q: &mut dyn Tab) {
    let from_p: Vec<(&str, Option<String>)> = KEYS.iter().map(|k| (*k, p.held(k))).collect();
    sync(q, p);
    for (key, frame) in from_p {
        if let Some(frame) = frame {
            q.arrive("board", key, &frame);
        }
    }
}

#[test]
fn a_convergent_field_is_its_plain_value_to_the_page_that_writes_it() {
    on_each_tier(BOARD, 1, |tier, mut tabs| {
        let p = &mut tabs[0];
        let doc = p.document();
        assert_eq!(
            shown(&doc, "likes"),
            "10",
            "{tier}: a counter starts from its starting total: {doc}"
        );
        p.click("#like");
        p.click("#like");
        p.click("#unlike");
        p.click("#ann");
        p.click("#bob");
        p.click("#drop-ann");
        p.click("#say-a");
        p.click("#say-b");
        let doc = p.document();
        assert_eq!(shown(&doc, "likes"), "11", "{tier}: {doc}");
        assert_eq!(items(&doc, "guests"), ["bob"], "{tier}: {doc}");
        assert_eq!(items(&doc, "lines"), ["a", "b"], "{tier}: {doc}");
    });
}

#[test]
fn concurrent_tally_increments_on_two_pages_both_count() {
    for p_first in [true, false] {
        on_each_tier(BOARD, 2, |tier, mut tabs| {
            let [p, q] = &mut tabs[..] else {
                unreachable!()
            };
            p.click("#like");
            q.click("#like");
            q.click("#like");
            if p_first {
                exchange(p.as_mut(), q.as_mut());
            } else {
                exchange(q.as_mut(), p.as_mut());
            }
            for (who, tab) in [("P", &mut *p), ("Q", &mut *q)] {
                let doc = tab.document();
                assert_eq!(
                    shown(&doc, "likes"),
                    "13",
                    "{tier}, page {who}: 10 + 1 + 2 — no increment lost: {doc}"
                );
            }
        });
    }
}

#[test]
fn a_concurrent_decrement_and_increment_both_count() {
    on_each_tier(BOARD, 2, |tier, mut tabs| {
        let [p, q] = &mut tabs[..] else {
            unreachable!()
        };
        p.click("#like");
        p.click("#like");
        q.click("#unlike");
        exchange(p.as_mut(), q.as_mut());
        for (who, tab) in [("P", &mut *p), ("Q", &mut *q)] {
            let doc = tab.document();
            assert_eq!(
                shown(&doc, "likes"),
                "11",
                "{tier}, page {who}: 10 + 2 - 1: {doc}"
            );
        }
    });
}

#[test]
fn a_pages_later_increments_replace_what_another_page_held_of_its_count() {
    on_each_tier(BOARD, 2, |tier, mut tabs| {
        let [p, q] = &mut tabs[..] else {
            unreachable!()
        };
        p.click("#like");
        exchange(p.as_mut(), q.as_mut());
        p.click("#like");
        p.click("#like");
        exchange(q.as_mut(), p.as_mut());
        for (who, tab) in [("P", &mut *p), ("Q", &mut *q)] {
            let doc = tab.document();
            assert_eq!(
                shown(&doc, "likes"),
                "13",
                "{tier}, page {who}: P's running total of 3 replaces the 1 Q held of it — kept, not added: {doc}"
            );
        }
    });
}

#[test]
fn a_convergent_count_refuses_to_go_down_and_keeps_what_it_had() {
    on_each_tier(BOARD, 1, |tier, mut tabs| {
        let p = &mut tabs[0];
        p.click("#visit");
        let before = p.held("board.visits");
        let failure = p
            .click_failing("#unvisit")
            .unwrap_or_else(|| panic!("{tier}: a ConvergentCount taken down fails"));
        assert!(
            failure.contains("ConvergentCount") && failure.contains("visits"),
            "{tier}: the failure names the field and what it is: {failure}"
        );
        let doc = p.document();
        assert_eq!(
            shown(&doc, "visits"),
            "1",
            "{tier}: the count is what its topic holds, not the decrease: {doc}"
        );
        assert_eq!(
            p.held("board.visits"),
            before,
            "{tier}: and nothing was published"
        );
    });
}

#[test]
fn concurrent_set_adds_on_two_pages_both_appear_in_every_order() {
    for p_first in [true, false] {
        on_each_tier(BOARD, 2, |tier, mut tabs| {
            let [p, q] = &mut tabs[..] else {
                unreachable!()
            };
            p.click("#ann");
            q.click("#bob");
            if p_first {
                exchange(p.as_mut(), q.as_mut());
            } else {
                exchange(q.as_mut(), p.as_mut());
            }
            for (who, tab) in [("P", &mut *p), ("Q", &mut *q)] {
                let doc = tab.document();
                assert_eq!(
                    items(&doc, "guests"),
                    ["ann", "bob"],
                    "{tier}, page {who}: both adds, in one order: {doc}"
                );
            }
        });
    }
}

#[test]
fn an_add_concurrent_with_a_remove_wins() {
    on_each_tier(BOARD, 2, |tier, mut tabs| {
        let [p, q] = &mut tabs[..] else {
            unreachable!()
        };
        p.click("#ann");
        p.click("#drop-ann");
        q.click("#ann");
        exchange(p.as_mut(), q.as_mut());
        for (who, tab) in [("P", &mut *p), ("Q", &mut *q)] {
            let doc = tab.document();
            assert_eq!(
                items(&doc, "guests"),
                ["ann"],
                "{tier}, page {who}: P's remove takes out only the add P had seen; Q's concurrent add stands: {doc}"
            );
        }
        p.click("#drop-ann");
        exchange(p.as_mut(), q.as_mut());
        for (who, tab) in [("P", &mut *p), ("Q", &mut *q)] {
            let doc = tab.document();
            assert_eq!(
                items(&doc, "guests"),
                Vec::<String>::new(),
                "{tier}, page {who}: a remove after seeing both adds takes both: {doc}"
            );
        }
    });
}

#[test]
fn a_set_of_numbers_is_in_numeric_order_on_every_page() {
    on_each_tier(BOARD, 2, |tier, mut tabs| {
        let [p, q] = &mut tabs[..] else {
            unreachable!()
        };
        p.click("#seats-in");
        exchange(p.as_mut(), q.as_mut());
        for (who, tab) in [("P", &mut *p), ("Q", &mut *q)] {
            let doc = tab.document();
            assert_eq!(
                items(&doc, "seats"),
                ["9", "10"],
                "{tier}, page {who}: {doc}"
            );
        }
    });
}

#[test]
fn concurrent_sequence_appends_both_appear_in_one_order_on_every_page() {
    for p_first in [true, false] {
        on_each_tier(BOARD, 2, |tier, mut tabs| {
            let [p, q] = &mut tabs[..] else {
                unreachable!()
            };
            p.click("#say-a");
            q.click("#say-b");
            if p_first {
                exchange(p.as_mut(), q.as_mut());
            } else {
                exchange(q.as_mut(), p.as_mut());
            }
            let on_p = items(&p.document(), "lines");
            let on_q = items(&q.document(), "lines");
            assert_eq!(on_p, on_q, "{tier}: the two pages hold one sequence");
            let mut sorted = on_p.clone();
            sorted.sort();
            assert_eq!(
                sorted,
                ["a", "b"],
                "{tier}: both appends are in it: {on_p:?}"
            );
        });
    }
}

#[test]
fn an_insert_after_a_line_another_page_is_extending_stays_in_place() {
    on_each_tier(BOARD, 2, |tier, mut tabs| {
        let [p, q] = &mut tabs[..] else {
            unreachable!()
        };
        p.click("#say-a");
        exchange(p.as_mut(), q.as_mut());
        p.click("#say-b");
        q.click("#say-c");
        exchange(p.as_mut(), q.as_mut());
        let on_p = items(&p.document(), "lines");
        let on_q = items(&q.document(), "lines");
        assert_eq!(on_p, on_q, "{tier}: one sequence");
        assert_eq!(
            on_p.first().map(String::as_str),
            Some("a"),
            "{tier}: what both saw first stays first: {on_p:?}"
        );
        assert_eq!(on_p.len(), 3, "{tier}: {on_p:?}");
    });
}

#[test]
fn lines_one_action_inserts_keep_their_order_on_every_page() {
    on_each_tier(BOARD, 2, |tier, mut tabs| {
        let [p, q] = &mut tabs[..] else {
            unreachable!()
        };
        p.click("#say-a");
        p.click("#say-both");
        exchange(p.as_mut(), q.as_mut());
        for (who, tab) in [("P", &mut *p), ("Q", &mut *q)] {
            let doc = tab.document();
            assert_eq!(
                items(&doc, "lines"),
                ["a", "x", "y"],
                "{tier}, page {who}: each inserted line follows the one before it: {doc}"
            );
        }
    });
}

#[test]
fn a_line_removed_on_one_page_is_gone_from_every_page_and_a_concurrent_append_stays() {
    on_each_tier(BOARD, 2, |tier, mut tabs| {
        let [p, q] = &mut tabs[..] else {
            unreachable!()
        };
        p.click("#say-a");
        p.click("#say-b");
        exchange(p.as_mut(), q.as_mut());
        q.click("#drop-line");
        p.click("#say-c");
        exchange(p.as_mut(), q.as_mut());
        for (who, tab) in [("P", &mut *p), ("Q", &mut *q)] {
            let doc = tab.document();
            assert_eq!(
                items(&doc, "lines"),
                ["b", "c"],
                "{tier}, page {who}: Q's removal and P's append both land: {doc}"
            );
        }
    });
}

#[test]
fn an_echo_of_a_convergent_frame_settles_after_one_round() {
    on_each_tier(BOARD, 2, |tier, mut tabs| {
        let [p, q] = &mut tabs[..] else {
            unreachable!()
        };
        p.click("#like");
        p.click("#ann");
        p.click("#say-a");
        p.published();
        q.published();
        sync(p.as_mut(), q.as_mut());
        let forwarded = q.published();
        assert!(
            !forwarded.is_empty(),
            "{tier}: the page that took the frames holds them where its relay reads them"
        );
        for (topic, key, frame) in &forwarded {
            p.arrive(topic, key, frame);
        }
        assert_eq!(
            p.published(),
            [],
            "{tier}: the echo back to its writer writes nothing"
        );
        assert_eq!(q.published(), [], "{tier}: and nothing is left to carry");
    });
}

/// ★ A PAGE WHOSE SHARED STORES ARE ALL PLAIN DRAWS NO IDENTITY. Only a convergent field tags its
/// operations with the page's replica id, so only a page with one binds it — a scalar-only page
/// stays free of any draw, and its frames are the same on every run.
#[test]
fn only_a_page_with_a_convergent_field_draws_a_replica_id() {
    const PLAIN: &str = "\
## Store Board shared on \"board\"
State:
    Let title be \"none\".

## Component App
Use Board.
View:
    <p id=\"t\">{Board's title}</p>
";
    let replica = logicaffeine_language::parser::lgx_lower::shared_replica_binding();
    for (src, draws) in [(PLAIN, false), (BOARD, true)] {
        dom::reset();
        with_parsed_program(src, |parsed, interner| {
            let (stmts, types, _) = parsed.expect("parses");
            let program =
                Compiler::compile_with_types(stmts, interner, Some(types)).expect("compiles");
            let mut session = Session::new(Vm::new(&program), interner);
            session.run_main().expect("main runs");
            assert_eq!(
                session.global_text(replica).is_some(),
                draws,
                "a page {} a convergent field {} a replica id",
                if draws { "with" } else { "without" },
                if draws { "binds" } else { "binds no" }
            );
        });
    }
}

/// ★ A CONVERGENT TYPE MEANS SOMETHING ONLY WHERE THE STATE IS SHARED, and anywhere else it is
/// refused by name rather than accepted and ignored.
#[test]
fn a_convergent_type_outside_a_shared_store_is_refused_by_name() {
    let refused = |src: &str| -> String {
        with_parsed_program(src, |parsed, _| match parsed {
            Ok(_) => panic!("must be refused:\n{src}"),
            Err(e) => e,
        })
    };
    let in_component = refused(
        "## Component App\nState:\n    Let likes: Tally be 0.\nView:\n    <p>{likes}</p>\n",
    );
    assert!(
        in_component.contains("likes") && in_component.contains("shared on"),
        "a component's state shares nothing: {in_component}"
    );
    let in_local_store = refused(
        "## Store Board\nState:\n    Let guests: SharedSet of Text be a new Set of Text.\n\n## Component App\nUse Board.\nView:\n    <p>x</p>\n",
    );
    assert!(
        in_local_store.contains("guests") && in_local_store.contains("shared on"),
        "neither does a local store's: {in_local_store}"
    );
    let not_whole = refused(
        "## Store Board shared on \"b\"\nState:\n    Let likes: Tally be \"lots\".\n\n## Component App\nUse Board.\nView:\n    <p>x</p>\n",
    );
    assert!(
        not_whole.contains("whole number"),
        "a counter starts from a whole number: {not_whole}"
    );
    let not_empty = refused(
        "## Store Board shared on \"b\"\nState:\n    Let lines: SharedSequence of Text be 3.\n\n## Component App\nUse Board.\nView:\n    <p>x</p>\n",
    );
    assert!(
        not_empty.contains("starts empty"),
        "a sequence starts empty: {not_empty}"
    );
    let unsupported = refused(
        "## Store Board shared on \"b\"\nState:\n    Let tags: SharedMap of Text and Int be 0.\n\n## Component App\nUse Board.\nView:\n    <p>x</p>\n",
    );
    assert!(
        unsupported.contains("SharedMap"),
        "a convergent type a shared store cannot hold is named: {unsupported}"
    );
}
