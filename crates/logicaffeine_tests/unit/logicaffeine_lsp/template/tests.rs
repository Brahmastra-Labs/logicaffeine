use super::*;

#[test]
fn a_longer_phrase_wins_over_its_prefix() {
    assert_eq!(tag_phrase("else if done:"), Some(("else if", 7)));
    assert_eq!(tag_phrase("else:"), Some(("else", 4)));
    assert_eq!(tag_phrase("  for each t in todos:"), Some(("for each", 10)));
}

#[test]
fn a_hole_that_starts_like_a_tag_word_is_still_a_hole() {
    assert_eq!(
        tag_phrase("ending"),
        None,
        "`ending` is a name, not `{{end}}`"
    );
    assert_eq!(tag_phrase("iffy"), None);
    assert_eq!(tag_phrase("count"), None);
}

#[test]
fn a_pipe_inside_a_string_or_a_call_is_not_a_filter() {
    assert_eq!(filter_pipes("price | money | pad 8"), vec![6, 14]);
    assert_eq!(filter_pipes("\"a|b\" | upper"), vec![6]);
    assert_eq!(filter_pipes("join(xs, \"|\") | upper"), vec![14]);
}
