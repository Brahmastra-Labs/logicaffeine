use super::*;

fn fed(text: &[u8]) -> Markup {
    let mut m = Markup::new();
    for &b in text {
        m.update(b);
    }
    m
}

fn name(s: &[u8]) -> u64 {
    s.iter().fold(0, |h, &b| hash2(h, u64::from(b.to_ascii_lowercase())))
}

#[test]
fn a_tag_is_read_through_its_name_attributes_and_values() {
    let steps: &[(&[u8], Place)] = &[
        (b"<", Place::TagName),
        (b"<a", Place::TagName),
        (b"<a ", Place::Tag),
        (b"<a h", Place::AttrName),
        (b"<a href", Place::AttrName),
        (b"<a href=", Place::AttrValue),
        (b"<a href=\"x y", Place::AttrValue),
        (b"<a href=\"x y\"", Place::Tag),
        (b"<a href=\"x y\" id=z", Place::AttrValue),
        (b"<a href=\"x y\" id=z ", Place::Tag),
        (b"<a href=\"x y\" id=z>", Place::Text),
    ];
    for (text, place) in steps {
        assert_eq!(fed(text).place(), *place, "after {:?}", String::from_utf8_lossy(text));
    }
    let m = fed(b"<a href=\"x>y\" id=z ");
    assert_eq!(m.place(), Place::Tag, "a quoted `>` is the value's, not the tag's end");
    assert_eq!(m.attrs, 2);
}

#[test]
fn elements_open_nest_and_close_by_name_in_any_case() {
    let m = fed(b"<HTML><body><p>one<b>two</B>");
    assert_eq!(m.open, vec![name(b"html"), name(b"body"), name(b"p")], "</B> closed <b>");
    assert_eq!(m.top(), name(b"p"));
    assert_eq!(m.closed, name(b"b"));
    let m = fed(b"<ul><li>a<li>b</ul>");
    assert_eq!(m.open, Vec::<u64>::new(), "closing an element closes the unclosed ones inside it");
    let m = fed(b"<div><br/><img src=x /></div><p>");
    assert_eq!(m.open, vec![name(b"p")], "self-closing tags open nothing");
    let m = fed(b"<div></span>");
    assert_eq!(m.open, vec![name(b"div")], "a close with nothing to match leaves the stack");
}

#[test]
fn comments_declarations_and_comparisons_are_not_elements() {
    let m = fed(b"<!DOCTYPE html><!-- <p> --><?xml version=\"1.0\"?>");
    assert_eq!((m.place(), m.depth()), (Place::Text, 0));
    let m = fed(b"if (a < b && c <= d) { x = a<<2; }");
    assert_eq!((m.place(), m.depth()), (Place::Text, 0), "`<` before a space or another `<` is no tag");
    let mut m = fed(b"<p>");
    m.update(b'<');
    m.update(b'x');
    for _ in 0..LONGEST_TAG {
        m.update(b'y');
    }
    assert_eq!(m.place(), Place::Text, "a tag that never closes is let go");
    assert_eq!(m.depth(), 1);
}

#[test]
fn the_stack_keeps_the_innermost_elements() {
    let mut text = Vec::new();
    for i in 0..DEPTH + 8 {
        text.extend_from_slice(format!("<e{i}>").as_bytes());
    }
    let m = fed(&text);
    assert_eq!(m.depth(), DEPTH);
    assert_eq!(m.top(), name(format!("e{}", DEPTH + 7).as_bytes()));
}

#[test]
fn a_closing_tag_s_context_names_the_element_it_closes() {
    // Two documents whose closing tags spell different elements: their first contexts differ at `</` already.
    let a = fed(b"<p>text</");
    let b = fed(b"<em>text</");
    assert_ne!(a.contexts()[0], b.contexts()[0]);
    // The same element open: the same context, whatever the text inside.
    assert_eq!(fed(b"<p>other words</").contexts()[0], a.contexts()[0]);
}
