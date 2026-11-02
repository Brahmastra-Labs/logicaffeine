//! The form's rules against the shared vectors the worker's vitest walks too
//! (`crates/logicaffeine_tests/tests/fixtures/lead_vectors.json`), so the page and the worker
//! cannot disagree about what a valid email or phone is.

use super::*;
use serde_json::Value;

fn vectors() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/logicaffeine_tests/tests/fixtures/lead_vectors.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).expect("lead_vectors.json parses")
}

fn cases(field: &str) -> Vec<Value> {
    let cases = vectors()[field].as_array().unwrap_or_else(|| panic!("the vectors have no {field} cases")).clone();
    assert!(!cases.is_empty(), "the vectors have no {field} cases");
    cases
}

/// The case's input: a string, or `None` for JSON null.
fn input(case: &Value) -> Option<String> {
    match &case["input"] {
        Value::Null => None,
        Value::String(s) => {
            if let Some(n) = case.get("input_length") {
                assert_eq!(s.chars().count() as u64, n.as_u64().unwrap(), "{}: the input is not the length it names", case["note"]);
            }
            Some(s.clone())
        }
        other => panic!("{}: the page only ever sends strings, not {other}", case["note"]),
    }
}

#[test]
fn the_vectors_are_the_shape_this_walker_reads() {
    let v = vectors();
    assert_eq!(v["version"], 1, "the vectors moved to a version this walker does not know");
    let mut shape: Vec<&str> = v["shape"].as_object().unwrap().keys().map(String::as_str).collect();
    shape.sort();
    assert_eq!(shape, ["email[]", "input_length", "phone[]", "trim_set"]);
    assert!(cases("email").len() >= 40 && cases("phone").len() >= 40, "the vectors hold too few cases");
    assert!(cases("email").iter().any(|c| c["valid"] == true) && cases("email").iter().any(|c| c["valid"] == false));
    assert!(cases("phone").iter().any(|c| c["valid"] == true) && cases("phone").iter().any(|c| c["valid"] == false));
}

/// The code points the vectors say are trimmed.
fn trim_set() -> Vec<(u32, u32)> {
    vectors()["trim_set"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            let entry = entry.as_str().unwrap();
            let point = |s: &str| u32::from_str_radix(s.trim_start_matches("U+"), 16).unwrap_or_else(|_| panic!("bad code point {s}"));
            match entry.split_once("..") {
                Some((from, to)) => (point(from), point(to)),
                None => (point(entry), point(entry)),
            }
        })
        .collect()
}

#[test]
fn trimming_removes_exactly_the_trim_set_from_either_end() {
    let set = trim_set();
    for code in 0..=0x10FFFFu32 {
        let Some(c) = char::from_u32(code) else { continue };
        let listed = set.iter().any(|&(from, to)| (from..=to).contains(&code));
        let wrapped = format!("{c}a{c}");
        let trimmed = trim_field(&wrapped);
        if listed {
            assert_eq!(trimmed, "a", "U+{code:04X} is in the trim set but is kept");
        } else {
            assert_eq!(trimmed, wrapped, "U+{code:04X} is not in the trim set but is trimmed");
        }
    }
    assert_eq!(trim_field("  a b  "), "a b", "a character inside is never trimmed");
}

#[test]
fn every_email_vector_agrees() {
    let mut failures = Vec::new();
    for case in cases("email") {
        let note = case["note"].as_str().unwrap_or("?");
        let got = validate_email(input(&case).as_deref());
        let agrees = match (&got, case["valid"].as_bool().unwrap()) {
            (Ok(normalized), true) => Some(normalized.as_str()) == case["normalized"].as_str(),
            (Err(err), false) => case["error"] == "invalid_email" && case["reason"] == err.reason(),
            _ => false,
        };
        if !agrees {
            failures.push(format!("{note}: {:?} gave {got:?}, the vectors say {case}", case["input"]));
        }
    }
    assert!(failures.is_empty(), "the page's email rule disagrees with the vectors:\n{}", failures.join("\n"));
}

#[test]
fn every_phone_vector_agrees() {
    let mut failures = Vec::new();
    for case in cases("phone") {
        let note = case["note"].as_str().unwrap_or("?");
        let got = validate_phone(input(&case).as_deref());
        let agrees = match (&got, case["valid"].as_bool().unwrap()) {
            (Ok(None), true) => case["normalized"].is_null() && case["digits"].is_null(),
            (Ok(Some(phone)), true) => {
                Some(phone.normalized.as_str()) == case["normalized"].as_str() && Some(phone.digits.as_str()) == case["digits"].as_str()
            }
            (Err(err), false) => case["error"] == "invalid_phone" && case["reason"] == err.reason(),
            _ => false,
        };
        if !agrees {
            failures.push(format!("{note}: {:?} gave {got:?}, the vectors say {case}", case["input"]));
        }
    }
    assert!(failures.is_empty(), "the page's phone rule disagrees with the vectors:\n{}", failures.join("\n"));
}

#[test]
fn every_error_says_what_to_do_in_words() {
    for err in [EmailError::Required, EmailError::Malformed] {
        assert!(err.message().ends_with('.') && err.message().len() > 12, "{err:?}: {:?}", err.message());
    }
    for err in [PhoneError::BadCharacter, PhoneError::MisplacedPlus, PhoneError::TooLong, PhoneError::TooFewDigits, PhoneError::TooManyDigits] {
        assert!(err.message().ends_with('.') && err.message().len() > 12, "{err:?}: {:?}", err.message());
    }
    assert_ne!(EmailError::Required.message(), EmailError::Malformed.message());
}

#[test]
fn a_valid_request_is_the_body_the_worker_reads() {
    let body = request_body(" Ada@Example.COM ", "", "token-1", "").expect("a valid request");
    assert_eq!(
        body,
        serde_json::json!({
            "email": "Ada@example.com",
            "phone": null,
            "turnstile_token": "token-1",
            "source": SOURCE,
            "website": "",
        })
    );
    let body = request_body("ada@example.com", " +1 (555) 123-4567 ", "token-2", "").unwrap();
    assert_eq!(body["phone"], "+1 (555) 123-4567", "the phone is sent trimmed, as typed");
    let mut keys: Vec<&str> = body.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(keys, ["email", "phone", "source", "turnstile_token", "website"]);
}

#[test]
fn the_honeypot_rides_in_the_body_untouched() {
    let body = request_body("ada@example.com", "", "t", "http://spam.example").unwrap();
    assert_eq!(body["website"], "http://spam.example", "the worker, not the page, decides what a filled honeypot means");
}

#[test]
fn an_invalid_request_names_every_bad_field() {
    let errors = request_body("not-an-email", "12", "t", "").expect_err("both fields are bad");
    assert_eq!(errors, FieldErrors { email: Some(EmailError::Malformed), phone: Some(PhoneError::TooFewDigits) });
    let errors = request_body("  ", "", "t", "").expect_err("the email is required");
    assert_eq!(errors, FieldErrors { email: Some(EmailError::Required), phone: None });
    let errors = request_body("ada@example.com", "555-CALL-NOW", "t", "").expect_err("the phone is bad");
    assert_eq!(errors, FieldErrors { email: None, phone: Some(PhoneError::BadCharacter) });
}

#[test]
fn the_source_is_one_the_worker_accepts() {
    // The worker takes a source of 1-64 characters from A-Z a-z 0-9 . _ / # - (services/leads).
    assert!(!SOURCE.is_empty() && SOURCE.len() <= 64);
    assert!(SOURCE.chars().all(|c| c.is_ascii_alphanumeric() || "._/#-".contains(c)), "{SOURCE:?}");
    assert_eq!(SOURCE, "pricing", "the contract names the pricing form's source \"pricing\"");
}

#[test]
fn the_worker_s_answers_read_as_outcomes() {
    assert_eq!(Outcome::of(202, r#"{"ok":true}"#), Outcome::Sent);
    assert_eq!(Outcome::of(400, r#"{"ok":false,"error":"invalid_email","reason":"malformed"}"#), Outcome::Invalid(FieldErrors { email: Some(EmailError::Malformed), phone: None }));
    assert_eq!(Outcome::of(400, r#"{"ok":false,"error":"invalid_email","reason":"required"}"#), Outcome::Invalid(FieldErrors { email: Some(EmailError::Required), phone: None }));
    assert_eq!(Outcome::of(400, r#"{"ok":false,"error":"invalid_phone","reason":"too_many_digits"}"#), Outcome::Invalid(FieldErrors { email: None, phone: Some(PhoneError::TooManyDigits) }));
    for (status, body) in [
        (400, r#"{"ok":false,"error":"invalid_source"}"#),
        (400, r#"{"ok":false,"error":"invalid_json"}"#),
        (400, r#"{"ok":false,"error":"invalid_phone","reason":"malformed"}"#),
        (403, r#"{"ok":false,"error":"turnstile_failed"}"#),
        (403, r#"{"ok":false,"error":"origin_not_allowed"}"#),
        (413, r#"{"ok":false,"error":"body_too_large"}"#),
        (429, r#"{"ok":false,"error":"rate_limited"}"#),
        (500, r#"{"ok":false,"error":"internal"}"#),
        (502, "<html>bad gateway</html>"),
        (202, "not json"),
        (200, r#"{"ok":true}"#),
    ] {
        assert_eq!(Outcome::of(status, body), Outcome::Failed, "{status} {body}");
    }
}
