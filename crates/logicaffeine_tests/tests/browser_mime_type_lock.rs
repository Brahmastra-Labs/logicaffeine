//! LOCK — the ONE mapping every in-process tier types a name-only file by is the browser's.
//!
//! `dropped_file_type_lock` holds Chrome, the tree-walker and the lowered VM to one table through
//! a real drop. This file holds the mapping those tiers share (`semantics::dom::browser_mime_type`)
//! to the same table directly, with no browser in the loop, and pins the rule a `File` value is
//! typed by: a name alone, or a record that declares no type, is typed from the name; a declared
//! type is the author's, whatever the name says.

#[path = "support/dropped_names.rs"]
mod dropped_names;

use dropped_names::{DESKTOP_DROPS, DROPS};
use logicaffeine_compile::interpreter::RuntimeValue;
use logicaffeine_compile::semantics::dom::{browser_mime_type, DroppedFile};

#[test]
fn the_shared_mapping_names_every_drop_as_chrome_does() {
    for (name, expected) in DROPS {
        assert_eq!(browser_mime_type(name), *expected, "{name:?}");
    }
}

/// An extension a desktop may name otherwise is typed by the table here, on every machine.
#[test]
fn the_shared_mapping_answers_the_table_for_an_extension_a_desktop_may_name_otherwise() {
    for (name, expected) in DESKTOP_DROPS {
        assert_eq!(browser_mime_type(name), *expected, "{name:?}");
    }
}

#[test]
fn the_extension_is_read_after_the_last_dot_whatever_its_case() {
    assert_eq!(browser_mime_type("A.Txt"), "text/plain");
    assert_eq!(browser_mime_type("a.b.c.PNG"), "image/png");
    assert_eq!(
        browser_mime_type("png"),
        "",
        "a name with no dot has no extension"
    );
    assert_eq!(
        browser_mime_type(".png"),
        "image/png",
        "a name that is only an extension is typed by it"
    );
    assert_eq!(
        browser_mime_type("a."),
        "",
        "a trailing dot opens an empty extension"
    );
    assert_eq!(browser_mime_type(""), "");
}

/// The first entry of the table naming an extension is its type: `mp3` is `audio/mpeg`, the
/// primary list's, not the later `audio/mp3`; `js` is `text/javascript`, not the secondary list's
/// `application/javascript`; `ico` is the secondary list's first, `image/x-icon`.
#[test]
fn the_first_entry_naming_an_extension_wins() {
    assert_eq!(browser_mime_type("a.mp3"), "audio/mpeg");
    assert_eq!(browser_mime_type("a.js"), "text/javascript");
    assert_eq!(browser_mime_type("a.png"), "image/png");
    assert_eq!(browser_mime_type("a.ico"), "image/x-icon");
    assert_eq!(browser_mime_type("a.gz"), "application/gzip");
}

#[test]
fn a_file_named_alone_is_typed_from_its_name() {
    let named = DroppedFile::named("notes.txt");
    assert_eq!(named.name, "notes.txt");
    assert_eq!(named.declared_type, None, "a name declares no type");
    assert_eq!(named.mime_type(), "text/plain");
    assert_eq!(named.bytes, Vec::<u8>::new());
    let unknown = DroppedFile::named("weird.qqq");
    assert_eq!(
        unknown.mime_type(),
        "",
        "an extension no browser knows has no type"
    );
}

#[test]
fn a_file_record_without_a_declared_type_is_typed_from_its_name() {
    let undeclared = DroppedFile::declared("a.json".to_string(), None, None, Some(vec![123, 125]))
        .expect("a well-formed file");
    assert_eq!(undeclared.declared_type, None);
    assert_eq!(undeclared.mime_type(), "application/json");
    assert_eq!(undeclared.bytes, vec![123, 125]);
    let sized = DroppedFile::declared("img.png".to_string(), Some(3), None, None)
        .expect("a well-formed file");
    assert_eq!(sized.mime_type(), "image/png");
    assert_eq!(sized.bytes, vec![0, 0, 0]);
}

#[test]
fn a_declared_type_is_the_authors_whatever_the_name_says() {
    let declared = DroppedFile::declared(
        "a.json".to_string(),
        None,
        Some("text/plain".to_string()),
        None,
    )
    .expect("a well-formed file");
    assert_eq!(declared.declared_type.as_deref(), Some("text/plain"));
    assert_eq!(declared.mime_type(), "text/plain");
    let typed_unknown = DroppedFile::declared(
        "weird.qqq".to_string(),
        None,
        Some("application/x-weird".to_string()),
        None,
    )
    .expect("a well-formed file");
    assert_eq!(typed_unknown.mime_type(), "application/x-weird");
    let typeless = DroppedFile::declared("notes.txt".to_string(), None, Some(String::new()), None)
        .expect("a well-formed file");
    assert_eq!(
        typeless.declared_type.as_deref(),
        Some(""),
        "a declared empty type is a declaration"
    );
    assert_eq!(typeless.mime_type(), "");
}

/// The `File` record a handler binds carries the effective type: what `file's type` reads.
#[test]
fn the_record_a_handler_binds_carries_the_effective_type() {
    let field = |file: &DroppedFile, name: &str| match file.to_value() {
        RuntimeValue::Struct(record) => {
            record.fields.get(name).map(RuntimeValue::to_display_string)
        }
        other => panic!("a File record, not {other:?}"),
    };
    let named = DroppedFile::named("notes.txt");
    assert_eq!(field(&named, "name").as_deref(), Some("notes.txt"));
    assert_eq!(field(&named, "type").as_deref(), Some("text/plain"));
    assert_eq!(field(&named, "size").as_deref(), Some("0"));
    let declared = DroppedFile::declared(
        "notes.txt".to_string(),
        None,
        Some("text/html".to_string()),
        None,
    )
    .expect("a well-formed file");
    assert_eq!(field(&declared, "type").as_deref(), Some("text/html"));
}
