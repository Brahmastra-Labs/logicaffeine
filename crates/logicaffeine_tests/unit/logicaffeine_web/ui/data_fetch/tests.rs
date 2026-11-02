use super::check_served;

const INDEX: &str = "<!DOCTYPE html>\n<html lang=\"en\">\n<head>";

#[test]
fn a_json_file_served_as_json_is_accepted() {
    assert_eq!(check_served("/data/lexicon.json", Some("application/json"), "{\"verbs\":[]}"), Ok(()));
}

#[test]
fn the_spa_fallback_page_is_reported_as_a_missing_file() {
    let err = check_served("/data/lexicon.json", Some("text/html; charset=utf-8"), INDEX).unwrap_err();
    assert!(err.contains("/data/lexicon.json") && err.contains("missing"), "{err}");
}

#[test]
fn the_fallback_is_caught_even_without_a_content_type() {
    assert!(check_served("/data/latest.json", None, INDEX).is_err());
    assert!(check_served("/data/latest.json", Some("application/octet-stream"), "  <!doctype html><html>").is_err());
}

#[test]
fn a_staged_html_file_is_html_and_is_accepted() {
    assert_eq!(check_served("/data/privacy.html", Some("text/html"), "<!DOCTYPE html><h1>Privacy</h1>"), Ok(()));
}

#[test]
fn a_json_body_behind_a_wrong_content_type_is_still_the_file() {
    assert_eq!(check_served("/data/solvers.json", Some("text/plain"), "[1,2,3]"), Ok(()));
}
