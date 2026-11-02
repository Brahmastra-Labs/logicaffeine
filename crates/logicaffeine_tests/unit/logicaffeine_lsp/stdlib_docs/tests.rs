use super::*;

#[test]
fn the_registry_covers_the_prelude_vocabulary() {
    for name in logicaffeine_compile::loader::prelude_vocabulary() {
        assert!(
            stdlib_doc(&name).is_some(),
            "{name}: in prelude_vocabulary but missing from the stdlib docs registry"
        );
    }
}

#[test]
fn md5_teaches_from_its_literate_note() {
    let entry = stdlib_doc("md5").expect("md5 is stdlib");
    assert!(entry.signature.contains("md5"), "{}", entry.signature);
    assert!(
        entry.doc.as_deref().is_some_and(|d| d.contains("MD5")),
        "md5's Note must teach: {:?}",
        entry.doc
    );
    assert!(!entry.is_type);
}

#[test]
fn definition_body_types_are_typed_entries() {
    let entry = stdlib_doc("Message").expect("net.lg defines Message");
    assert!(entry.is_type);
    assert!(
        entry.doc.is_some(),
        "the Definition block's Note documents Message"
    );
}

#[test]
fn hover_md_carries_signature_and_prose() {
    let entry = stdlib_doc("read").expect("file.lg defines read");
    let md = hover_md("read", entry);
    assert!(md.contains("standard library"), "{md}");
    assert!(md.contains("## To native read"), "{md}");
    assert!(md.contains("Reads a whole file"), "{md}");
}
