//! Host-held callbacks release their storage and captures without reusing stale identities.
#![cfg(feature = "wasm-jit")]
#[path = "compile_common/mod.rs"]
mod common;
use common::wasm_host::mount_interactive_checked;

fn churn(list_capture: bool, registrations: usize, check_stale: bool) {
    let binding = if list_capture {
        "        Let payload be [73 + n, 74].\n        Let callback be (event: Int) -> item 1 of payload.\n"
    } else {
        "        Let payload be 73 + n.\n        Let callback be (event: Int) -> payload.\n"
    };
    let second = if registrations == 2 {
        "        Let second be onWindowEvent(\"offline\", callback).\n"
    } else {
        ""
    };
    let source = format!("## Component App\nState:\n    Let n be 0.\nActions:\n    To replace:\n{binding}        Let first be onWindowEvent(\"resize\", callback).\n{second}        Set n to 1 - n.\nView:\n    <div><button id=\"a\" on-click={{replace}}>a</button><p>{{n}}</p></div>\n\n## Main\nMount App.\n");
    let mut page = mount_interactive_checked(&source);
    let button = *page.listening("click").last().expect("replacement action");
    let mut stale = None;
    let mut warm = 0;
    for turn in 1..=1000 {
        let before = page.handlers().len();
        page.dispatch(button, "click", "")
            .unwrap_or_else(|e| panic!("turn {turn}: {e}\n{source}"));
        page.safepoint();
        let handlers = page.handlers();
        let added = &handlers[before..];
        assert_eq!(
            added.len(),
            registrations,
            "one independent hold per registration"
        );
        let expected = 73 + ((turn - 1) % 2) as i64;
        for &handler in added {
            assert_eq!(
                page.call_closure(handler, 0).expect("live callback"),
                expected
            );
        }
        if check_stale {
            if let Some(old) = stale {
                assert!(
                    page.call_closure(old, 0).is_err(),
                    "released callback became callable again on turn {turn}"
                );
            }
        }
        page.safepoint();
        assert_eq!(
            page.census(),
            0,
            "turn {turn}: host roots and captures are counted exactly"
        );
        for (index, &handler) in added.iter().enumerate() {
            page.release_handle(handler);
            if let Some(&still_live) = added.get(index + 1) {
                assert_eq!(
                    page.call_closure(still_live, 0)
                        .expect("other registration still holds its callback"),
                    expected
                );
            }
        }
        stale.get_or_insert(added[0]);
        page.safepoint();
        assert_eq!(
            page.census(),
            0,
            "turn {turn}: released registrations own nothing"
        );
        if turn == 100 {
            warm = page.heap_allocated();
        }
    }
    assert_eq!(
        page.heap_allocated(),
        warm,
        "a bounded number of live callbacks must use bounded storage"
    );
}

#[test]
fn released_scalar_callbacks_reuse_storage() {
    churn(false, 1, false);
}
#[test]
fn released_callbacks_release_captured_lists() {
    churn(true, 1, false);
}
#[test]
fn duplicate_registrations_hold_their_callback_independently() {
    churn(true, 2, false);
}
#[test]
fn released_callback_identities_never_rebind_after_collection() {
    churn(true, 1, true);
}
