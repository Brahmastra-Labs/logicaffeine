//! ★ LOCK — a `shared on` field CONVERGES: pages that have seen the same frames hold the same value,
//! whatever order the frames reached them in. ★
//!
//! A shared store's field lives in its topic as a stamped frame `<lamport>:<value>`. A page publishes
//! a frame when an action moves the field, and a relay hands every frame to every other page on the
//! topic through the page's `__lgx_shared(topic, key, frame)`. That entry keeps the frame iff it
//! orders after the one held — by Lamport time, then by the value's bytes — so the held frame is
//! the greatest frame the page has seen. Greatest-of-a-set is commutative, associative and
//! idempotent: delivery order, duplicates and echoes cannot make two pages disagree.
//!
//! Every row runs on BOTH lowered engines, each tab a page of its own (`support/shared_tabs.rs`),
//! with this file playing the relay.

#[path = "support/shared_tabs.rs"]
mod shared_tabs;

use shared_tabs::{on_each_tier, on_each_tier_subscribed, shown, Tab};

/// One store on one topic, with an action per value it can be moved to.
const BOARD: &str = "\
## Store Board shared on \"board\"
State:
    Let title be \"none\".
    Let votes be 0.
Actions:
    To apple:
        Set title to \"apple\".
    To banana:
        Set title to \"banana\".
    To vote:
        Set votes to votes + 1.

## Component App
Use Board.
Actions:
    To pickApple:
        Call Board's apple.
    To pickBanana:
        Call Board's banana.
    To addVote:
        Call Board's vote.
View:
    <p id=\"t\">{Board's title}</p>
    <p id=\"v\">{Board's votes}</p>
    <button id=\"a\" on-click={pickApple}>a</button>
    <button id=\"b\" on-click={pickBanana}>b</button>
    <button id=\"c\" on-click={addVote}>c</button>
";

/// Two stores on one topic in one page — one shared store under two names.
const TWO_REPLICAS: &str = "\
## Store Alpha shared on \"room\"
State:
    Let notes be 0.

## Store Beta shared on \"room\"
State:
    Let notes be 0.

## Component App
Use Alpha.
Use Beta.
View:
    <p id=\"a\">{Alpha's notes}</p>
    <p id=\"b\">{Beta's notes}</p>
";

const TITLE: &str = "board.title";
const VOTES: &str = "board.votes";

/// Hand every frame `from` holds for `keys` to `to`, as a relay syncing a page does.
fn sync(from: &mut dyn Tab, to: &mut dyn Tab, keys: &[&str]) {
    for key in keys {
        if let Some(frame) = from.held(key) {
            to.arrive("board", key, &frame);
        }
    }
}

#[test]
fn a_joining_page_publishes_what_the_topic_does_not_hold_yet() {
    on_each_tier(BOARD, 1, |tier, mut tabs| {
        let p = &mut tabs[0];
        assert_eq!(
            p.held(TITLE).as_deref(),
            Some("1:none"),
            "{tier}: the first field, stamped at the first tick"
        );
        assert_eq!(
            p.held(VOTES).as_deref(),
            Some("2:0"),
            "{tier}: the second field, at the next"
        );
        let doc = p.document();
        assert_eq!(shown(&doc, "t"), "none", "{tier}: {doc}");
    });
}

#[test]
fn a_page_opening_on_a_topic_adopts_what_it_holds() {
    on_each_tier_subscribed(BOARD, 1, &[(TITLE, "9:hello")], |tier, mut tabs| {
        let p = &mut tabs[0];
        let doc = p.document();
        assert_eq!(
            shown(&doc, "t"),
            "hello",
            "{tier}: the joining page draws the topic's value, not its own initializer: {doc}"
        );
        assert_eq!(
            shown(&doc, "v"),
            "0",
            "{tier}: a field the topic does not hold starts from the initializer: {doc}"
        );
        assert_eq!(
            p.held(TITLE).as_deref(),
            Some("9:hello"),
            "{tier}: adopting a frame does not restamp it"
        );
        assert_eq!(
            p.held(VOTES).as_deref(),
            Some("10:0"),
            "{tier}: the field the topic lacked is published, stamped after the frame the page adopted"
        );
        p.click("#a");
        assert_eq!(
            p.held(TITLE).as_deref(),
            Some("11:apple"),
            "{tier}: and every later write orders after both"
        );
    });
}

#[test]
fn a_newer_frame_is_kept_and_repaints_the_page() {
    on_each_tier(BOARD, 2, |tier, mut tabs| {
        let [p, q] = &mut tabs[..] else {
            unreachable!()
        };
        p.click("#a");
        assert_eq!(
            p.held(TITLE).as_deref(),
            Some("3:apple"),
            "{tier}: the action published its write"
        );
        sync(p.as_mut(), q.as_mut(), &[TITLE]);
        assert_eq!(
            q.held(TITLE).as_deref(),
            Some("3:apple"),
            "{tier}: the other page kept the newer frame"
        );
        let doc = q.document();
        assert_eq!(
            shown(&doc, "t"),
            "apple",
            "{tier}: and drew it without any action of its own: {doc}"
        );
    });
}

#[test]
fn an_older_frame_is_ignored() {
    on_each_tier(BOARD, 2, |tier, mut tabs| {
        let [p, q] = &mut tabs[..] else {
            unreachable!()
        };
        p.click("#a");
        let apple = p.held(TITLE).expect("published");
        p.click("#b");
        let banana = p.held(TITLE).expect("published");
        assert_eq!(
            (apple.as_str(), banana.as_str()),
            ("3:apple", "4:banana"),
            "{tier}"
        );
        q.arrive("board", TITLE, &banana);
        q.published();
        q.arrive("board", TITLE, &apple);
        assert_eq!(
            q.held(TITLE),
            Some(banana),
            "{tier}: a frame that arrives late does not undo a later write"
        );
        assert_eq!(q.published(), [], "{tier}: and ignoring it writes nothing");
        let doc = q.document();
        assert_eq!(shown(&doc, "t"), "banana", "{tier}: {doc}");
    });
}

#[test]
fn concurrent_writes_at_one_lamport_time_resolve_the_same_way_on_every_page() {
    on_each_tier(BOARD, 2, |tier, mut tabs| {
        let [p, q] = &mut tabs[..] else {
            unreachable!()
        };
        p.click("#a");
        q.click("#b");
        assert_eq!(p.held(TITLE).as_deref(), Some("3:apple"), "{tier}");
        assert_eq!(
            q.held(TITLE).as_deref(),
            Some("3:banana"),
            "{tier}: the two writes are concurrent"
        );
        let from_p = p.held(TITLE).unwrap();
        let from_q = q.held(TITLE).unwrap();
        p.arrive("board", TITLE, &from_q);
        q.arrive("board", TITLE, &from_p);
        assert_eq!(
            p.held(TITLE).as_deref(),
            Some("3:banana"),
            "{tier}: the value that orders after wins"
        );
        assert_eq!(
            q.held(TITLE).as_deref(),
            Some("3:banana"),
            "{tier}: on BOTH pages"
        );
        assert_eq!(
            p.document(),
            q.document(),
            "{tier}: the two pages draw the same page"
        );
    });
}

#[test]
fn pages_that_see_the_same_frames_in_any_order_hold_the_same_value() {
    let frames = ["6:zzz", "7:m", "7:q", "7:"];
    let mut orders: Vec<Vec<&str>> = vec![Vec::new()];
    for _ in 0..frames.len() {
        orders = orders
            .into_iter()
            .flat_map(|order| {
                frames
                    .iter()
                    .filter(|f| !order.contains(f))
                    .map(|f| {
                        let mut next = order.clone();
                        next.push(f);
                        next
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
    }
    assert_eq!(orders.len(), 24, "every order of the four frames");
    on_each_tier(BOARD, orders.len(), |tier, mut tabs| {
        for (order, tab) in orders.iter().zip(tabs.iter_mut()) {
            for frame in order {
                tab.arrive("board", TITLE, frame);
            }
            assert_eq!(
                tab.held(TITLE).as_deref(),
                Some("7:q"),
                "{tier}: delivered in the order {order:?}"
            );
            let doc = tab.document();
            assert_eq!(
                shown(&doc, "t"),
                "q",
                "{tier}: delivered in the order {order:?}: {doc}"
            );
        }
    });
}

#[test]
fn a_frame_value_holding_a_colon_or_nothing_arrives_whole() {
    on_each_tier(BOARD, 1, |tier, mut tabs| {
        let p = &mut tabs[0];
        p.arrive("board", TITLE, "9:a:b");
        let doc = p.document();
        assert_eq!(
            shown(&doc, "t"),
            "a:b",
            "{tier}: the value is everything after the stamp: {doc}"
        );
        p.arrive("board", TITLE, "10:");
        let doc = p.document();
        assert_eq!(
            shown(&doc, "t"),
            "",
            "{tier}: and an empty value is a value: {doc}"
        );
        assert_eq!(p.held(TITLE).as_deref(), Some("10:"), "{tier}");
    });
}

#[test]
fn a_frame_seen_moves_the_clock_so_the_next_write_orders_after_it() {
    on_each_tier(BOARD, 2, |tier, mut tabs| {
        let [p, q] = &mut tabs[..] else {
            unreachable!()
        };
        q.arrive("board", TITLE, "40:apple");
        q.click("#b");
        assert_eq!(
            q.held(TITLE).as_deref(),
            Some("41:banana"),
            "{tier}: a write after seeing time 40 is stamped after it, or it would lose to the frame it replaced"
        );
        p.click("#a");
        sync(q.as_mut(), p.as_mut(), &[TITLE]);
        let doc = p.document();
        assert_eq!(shown(&doc, "t"), "banana", "{tier}: {doc}");
    });
}

#[test]
fn a_field_an_action_did_not_move_is_not_published() {
    on_each_tier(BOARD, 1, |tier, mut tabs| {
        let p = &mut tabs[0];
        p.arrive("board", TITLE, "5:apple");
        p.click("#c");
        assert_eq!(
            p.held(VOTES).as_deref(),
            Some("6:1"),
            "{tier}: the moved field is published"
        );
        assert_eq!(
            p.held(TITLE).as_deref(),
            Some("5:apple"),
            "{tier}: the untouched one keeps the frame it adopted — restamping it would claim a newer \
             write of a value this page never wrote, and override a concurrent write to it"
        );
        p.click("#a");
        assert_eq!(
            p.held(TITLE).as_deref(),
            Some("5:apple"),
            "{tier}: a write of the value already held moves nothing"
        );
    });
}

/// A page hosted in process has no other page to carry its frames to, so what it publishes is
/// not left waiting for a relay that never comes.
#[test]
fn a_page_hosted_alone_leaves_no_frames_waiting() {
    on_each_tier(BOARD, 1, |tier, mut tabs| {
        let p = &mut tabs[0];
        p.click("#a");
        p.click("#c");
        assert_eq!(
            p.held(TITLE).as_deref(),
            Some("3:apple"),
            "{tier}: the click published"
        );
        assert_eq!(p.published(), [], "{tier}: and the host took its frames");
    });
}

#[test]
fn an_echo_settles_after_one_round() {
    on_each_tier(BOARD, 2, |tier, mut tabs| {
        let [p, q] = &mut tabs[..] else {
            unreachable!()
        };
        p.click("#a");
        p.published();
        q.published();
        sync(p.as_mut(), q.as_mut(), &[TITLE]);
        let forwarded = q.published();
        assert_eq!(
            forwarded,
            [(
                "board".to_string(),
                TITLE.to_string(),
                "3:apple".to_string()
            )],
            "{tier}: the page that kept the frame holds it where its relay reads it"
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
        assert_eq!(p.held(TITLE).as_deref(), Some("3:apple"), "{tier}");
    });
}

#[test]
fn a_frame_on_another_topic_leaves_this_one_alone() {
    on_each_tier(BOARD, 1, |tier, mut tabs| {
        let p = &mut tabs[0];
        let before = p.document();
        p.arrive("elsewhere", "elsewhere.title", "50:intruder");
        assert_eq!(p.held(TITLE).as_deref(), Some("1:none"), "{tier}");
        assert_eq!(
            p.document(),
            before,
            "{tier}: another topic's frame draws nothing here"
        );
    });
}

#[test]
fn every_replica_of_the_topic_in_a_page_takes_an_arrival() {
    on_each_tier(TWO_REPLICAS, 1, |tier, mut tabs| {
        let p = &mut tabs[0];
        p.arrive("room", "room.notes", "9:5");
        let doc = p.document();
        assert_eq!(shown(&doc, "a"), "5", "{tier}: {doc}");
        assert_eq!(
            shown(&doc, "b"),
            "5",
            "{tier}: both names of the one shared store: {doc}"
        );
    });
}
