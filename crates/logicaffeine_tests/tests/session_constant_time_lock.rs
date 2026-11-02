//! A session cookie's signature and a form's CSRF token are checked against the value the server
//! computes, and the visitor chooses what is checked. Text equality stops at the first byte that
//! differs, so the time a forged signature takes to refuse says how much of its front is right, and a
//! forger who can time refusals learns the signature a byte at a time. `session.lg` compares both with
//! `sameTextInConstantTime`, which reads every byte of texts of one length before it answers.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::run_all_tiers;

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::interpreter::{clear_fuel, install_fuel};

const SESSION_LG: &str = include_str!("../../logicaffeine_compile/assets/std/session.lg");

/// The body of `## To <name> …`: its lines up to the next `## ` header.
fn body_of(name: &str) -> String {
    let head = format!("## To {name} ");
    let start = SESSION_LG
        .find(&head)
        .unwrap_or_else(|| panic!("session.lg defines {name}"));
    let rest = &SESSION_LG[start + head.len()..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    rest[..end].to_string()
}

#[test]
fn the_signature_checks_compare_with_the_constant_time_helper() {
    let verify = body_of("verifySession");
    assert!(
        verify.contains("sameTextInConstantTime(mac, macHex(secret, value))"),
        "verifySession compares the signature byte for byte:\n{verify}"
    );
    let csrf = body_of("csrfValid");
    assert!(
        csrf.contains("sameTextInConstantTime(token, csrfToken(secret, session))"),
        "csrfValid compares the token byte for byte:\n{csrf}"
    );
    for (name, body) in [("verifySession", &verify), ("csrfValid", &csrf)] {
        for line in body.lines() {
            let compares_a_secret = ["mac", "token", "macHex", "csrfToken"].iter().any(|w| {
                line.contains(&format!("{w} is equal to"))
                    || line.contains(&format!("is equal to {w}"))
            });
            assert!(
                !compares_a_secret,
                "{name} compares a signature with text equality: `{}`",
                line.trim()
            );
        }
    }
}

#[test]
fn same_text_in_constant_time_answers_as_equality_does() {
    let pairs = [
        ("", ""),
        ("a", "a"),
        ("a", "b"),
        ("ab", "a"),
        ("a", "ab"),
        ("", "x"),
        ("0123456789abcdef", "0123456789abcdef"),
        ("0123456789abcdef", "1123456789abcdef"),
        ("0123456789abcdef", "0123456789abcdee"),
        ("héllo", "héllo"),
        ("héllo", "hello"),
        ("é", "e\u{301}"),
    ];
    let mut src = String::from("## Main\n");
    let mut expected = String::new();
    for (a, b) in pairs {
        src.push_str(&format!("Show sameTextInConstantTime({a:?}, {b:?}).\n"));
        expected.push_str(&format!("{}\n", a == b));
    }
    let r = run_all_tiers(&src);
    assert_eq!(r.error, None, "the program runs\n{src}");
    assert_eq!(r.output.trim(), expected.trim(), "\n{src}");
}

/// The steps the tree-walker takes to run `main`: it spends one unit of fuel a step, so a run that
/// finishes leaves the budget less the steps it took.
fn steps(main: &str) -> u64 {
    let src = format!("## Main\n{main}");
    let fuel = install_fuel(1 << 40);
    let run = tw_outcome(&src);
    let used = fuel.budget - fuel.remaining.get();
    clear_fuel();
    assert_eq!(run.error, None, "the program runs\n{src}");
    used
}

#[test]
fn the_step_count_reads_a_programs_work() {
    let one =
        steps("Let mutable s be 0.\nRepeat for i from 1 to 10:\n    Set s to s + i.\nShow s.\n");
    let two =
        steps("Let mutable s be 0.\nRepeat for i from 1 to 20:\n    Set s to s + i.\nShow s.\n");
    assert!(one > 10, "ten iterations take more than ten steps ({one})");
    assert!(
        two > one,
        "twenty iterations take more steps than ten ({two} against {one})"
    );
}

#[test]
fn the_step_count_sees_a_comparison_that_stops_early() {
    let early = "## To earlyEqual (a: Text) and (b: Text) -> Bool:\n    Let x be text_bytes(a).\n    Let y be text_bytes(b).\n    If (length of x) is not equal to (length of y):\n        Return false.\n    Repeat for i from 1 to length of x:\n        If (item i of x) is not equal to (item i of y):\n            Return false.\n    Return true.\n\n";
    let count = |b: &str| {
        let src = format!("{early}## Main\nShow earlyEqual(\"abcdefgh\", {b:?}).\n");
        let fuel = install_fuel(1 << 40);
        let run = tw_outcome(&src);
        let used = fuel.budget - fuel.remaining.get();
        clear_fuel();
        assert_eq!(run.error, None, "\n{src}");
        used
    };
    assert!(
        count("Xbcdefgh") < count("abcdefgX"),
        "a comparison that returns at the first difference takes fewer steps the earlier it differs"
    );
}

#[test]
fn same_text_in_constant_time_takes_as_many_steps_wherever_the_texts_differ() {
    let mac = "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843";
    let mut wrong_first = String::from("6");
    wrong_first.push_str(&mac[1..]);
    let mut wrong_last = mac[..63].to_string();
    wrong_last.push('4');
    let base = steps(&format!("Show sameTextInConstantTime({mac:?}, {mac:?}).\n"));
    for other in [&wrong_first, &wrong_last] {
        assert_eq!(
            steps(&format!(
                "Show sameTextInConstantTime({mac:?}, {other:?}).\n"
            )),
            base,
            "the comparison stops early on {other}"
        );
    }
}

/// `mac` with its hex digit at `at` replaced by another.
fn wrong_at(mac: &str, at: usize) -> String {
    let mut digits: Vec<char> = mac.chars().collect();
    digits[at] = if digits[at] == '0' { '1' } else { '0' };
    digits.into_iter().collect()
}

#[test]
fn a_session_signature_is_refused_in_as_many_steps_wherever_it_is_wrong() {
    let mac: String = logicaffeine_compile::crypto_accel::hmac_sha256(b"s3cret", b"ada")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let verify = |cookie: &str| {
        steps(&format!(
            "Inspect verifySession(\"s3cret\", {cookie:?}):\n    When Some (v):\n        Show v.\n    When None:\n        Show \"refused\".\n"
        ))
    };
    let first = verify(&format!("ada|{}", wrong_at(&mac, 0)));
    let last = verify(&format!("ada|{}", wrong_at(&mac, 63)));
    assert_eq!(
        first, last,
        "a signature wrong in its first byte is refused in as many steps as one wrong in its last"
    );
}

#[test]
fn a_csrf_token_is_refused_in_as_many_steps_wherever_it_is_wrong() {
    let mut key = b"s3cret".to_vec();
    key.extend_from_slice(b"|csrf");
    let token: String = logicaffeine_compile::crypto_accel::hmac_sha256(&key, b"ada")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let check = |t: &str| steps(&format!("Show csrfValid(\"s3cret\", \"ada\", {t:?}).\n"));
    assert_eq!(
        check(&wrong_at(&token, 0)),
        check(&wrong_at(&token, 63)),
        "a token wrong in its first byte is refused in as many steps as one wrong in its last"
    );
}
