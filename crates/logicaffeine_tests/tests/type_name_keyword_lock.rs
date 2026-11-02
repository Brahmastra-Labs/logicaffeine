//! ★ LOCK — a keyword-shaped word may NAME a type and may be USED as one, in every slot. ★
//!
//! LOGOS keywords are contextual, never reserved: `Note that …` opens an observation and `Note` is
//! also an ordinary noun — the obvious name for a record in half the applications anybody writes
//! first. Both readings are unambiguous, because a keyword only means something at the START of a
//! block or a sentence.
//!
//! ★ WHAT WAS ACTUALLY BROKEN, established by removing the fix and re-measuring rather than by
//! reading the code. Two slots accepted such a word and three refused it:
//!
//! ```text
//!   ## A Note has:                    accepted        (record NAME)
//!   a inner, which is Note.           accepted        (record FIELD's type)
//!   ## To titleOf (n: Note) -> Text   REFUSED         (parameter type)
//!   ## To mk () -> Note:              REFUSED         (RETURN type)
//!   (xs: Seq of Note)                 REFUSED         (generic argument)
//! ```
//!
//! A notes app meets all three the moment it writes `## To loadNotes () -> Seq of Note:`.
//!
//! ★ THE CAUSE IS THE ONE THIS CAMPAIGN KEEPS FINDING: two hand-maintained lists of the same
//! decision. `expect_identifier` held a list of freed keyword tokens and `consume_type_name` held
//! a second, shorter one, and they drifted — `Note`, `Record`, `Warn`, `During`, `Swap`, `Read`,
//! `Write`, `Merge`, `Add`, `Remove`, `First`, `Both`, `Either`, `Some`, `All` could every one of
//! them NAME a parameter and not one of them could TYPE one. The reader is told their noun is not
//! a noun, with nothing anywhere pointing at the collision, so they edit the field list, the
//! header, the indentation, and finally rename the type by guesswork.
//!
//! `consume_type_name` now asks the same predicate `expect_identifier` asks — "is this token's
//! lexeme a plain identifier word?" — instead of carrying its own list. A fix that lands in one
//! naming position and not the others is how a language ends up with an arbitrary set of words you
//! cannot use.
//!
//! ★ AND THE HEADER THIS REPLACES WAS STALE. It opened *"`## A Note has:` does not parse"*, which
//! had stopped being true: the record-NAME slot was closed earlier by `record_field_names_lock`
//! and nobody updated the story here. A lock whose stated premise no longer holds is a lock nobody
//! can reason from — found by auditing the claim instead of trusting it.
//!
//! Found by writing an ordinary notes app: the type is called `Note`, and it could not be.

use logicaffeine_compile::compile::tw_outcome;

/// A record named after a keyword, used the way any record is used.
fn program(type_name: &str) -> String {
    format!(
        "## A {type_name} has:\n\
         \x20   an id, which is Int.\n\
         \x20   a title, which is Text.\n\n\
         ## Main\n\
         Let x be a new {type_name} with id 1 and title \"hi\".\n\
         Show x's title.\n"
    )
}

/// The control: an ordinary noun nobody has claimed.
#[test]
fn a_plain_record_name_works() {
    let out = tw_outcome(&program("Memo"));
    assert_eq!(out.error, None, "the control must run");
    assert_eq!(out.output.trim(), "hi");
}

/// ★ THE LOAD-BEARING TEST — `Note` is a noun, and a record may be called one.
#[test]
fn a_record_may_be_named_note() {
    let out = tw_outcome(&program("Note"));
    assert_eq!(
        out.error, None,
        "`## A Note has:` is refused because `Note that …` is an observation statement, so the \
         type-name slot meets a keyword token. The author is told their noun is not a noun."
    );
    assert_eq!(
        out.output.trim(),
        "hi",
        "and it behaves like any other record"
    );
}

/// The other block keywords a person would reasonably name a record after. Each is an ordinary
/// English noun, and the header form (`## A <Name> has:`) is unambiguous in every case — the
/// keyword only means something at the START of a block or a sentence.
#[test]
fn the_other_ordinary_nouns_the_language_has_claimed() {
    for name in ["Example", "Policy", "Property", "Theory", "Main"] {
        let out = tw_outcome(&program(name));
        assert_eq!(
            out.error, None,
            "`## A {name} has:` must declare a record. `{name}` is a noun before it is a keyword, \
             and the two are never ambiguous in this position."
        );
    }
}

/// ★ THE SLOTS THAT ACTUALLY FAILED — a parameter TYPE, a RETURN type, and a GENERIC argument.
///
/// Each is checked in a PLAIN function, with no component anywhere, because that is where the
/// defect lives: `consume_type_name`, which every type position routes through. An earlier version
/// of this lock tested only a component header and so described the bug as a component problem.
#[test]
fn a_keyword_name_may_type_a_parameter_a_return_and_a_generic() {
    let decl = "## A Note has:\n\x20   an id, which is Int.\n\x20   a title, which is Text.\n\n";

    let param = format!(
        "{decl}## To titleOf (n: Note) -> Text:\n\x20   Return n's title.\n\n\
         ## Main\nShow titleOf(a new Note with id 1 and title \"hi\").\n"
    );
    assert_eq!(
        tw_outcome(&param).error,
        None,
        "a PARAMETER may be typed `Note`:\n{param}"
    );
    assert_eq!(tw_outcome(&param).output.trim(), "hi");

    let ret = format!(
        "{decl}## To mk () -> Note:\n\x20   Return a new Note with id 1 and title \"hi\".\n\n\
         ## Main\nShow mk()'s title.\n"
    );
    assert_eq!(ret.is_empty(), false);
    assert_eq!(
        tw_outcome(&ret).error,
        None,
        "a RETURN type may be `Note`:\n{ret}"
    );
    assert_eq!(tw_outcome(&ret).output.trim(), "hi");

    let generic = format!(
        "{decl}## To countN (xs: Seq of Note) -> Int:\n\x20   Return length of xs.\n\n\
         ## Main\nShow countN([a new Note with id 1 and title \"hi\"]).\n"
    );
    assert_eq!(
        tw_outcome(&generic).error,
        None,
        "a GENERIC argument may be `Note`:\n{generic}"
    );
    assert_eq!(tw_outcome(&generic).output.trim(), "1");
}

/// The same word typing a COMPONENT parameter — the shape the notes app used.
///
/// ★ THE FIXTURE MOUNTS A PARENT, and the first version of this test did not. It wrote
/// `Mount Row.` against a component declaring a prop, which fails — correctly, and for a reason
/// that has nothing to do with keywords: a root mount renders into the document, so there is no
/// caller to supply a prop, and a NON-keyword name (`m: Memo`) failed identically. The test was
/// red while proving nothing about its own subject. A component with props is a piece a page
/// PLACES, so the fixture places it.
#[test]
fn a_component_parameter_may_be_typed_with_a_keyword_name() {
    let src = "## A Note has:\n\
        \x20   an id, which is Int.\n\
        \x20   a title, which is Text.\n\n\
        ## Component Row (n: Note)\n\
        View:\n\
        \x20   <li>{n's title}</li>\n\n\
        ## Component App\n\
        State:\n\
        \x20   Let one be a new Note with id 1 and title \"hi\".\n\
        View:\n\
        \x20   <ul><Row n={one}/></ul>\n\n\
        ## Main\n\
        Mount App.\n";
    assert_eq!(
        tw_outcome(src).error,
        None,
        "a component parameter typed `Note` must be accepted — the record itself already is:\n{src}"
    );
}

/// And a parameter NAMED after one, which is the other half of the same freedom.
///
/// This one was NEVER broken — `expect_identifier` has always accepted `Note` in a name position.
/// It is kept because the two halves are one property, and because the earlier fixture's failure
/// was read as evidence that it WAS broken.
#[test]
fn a_component_parameter_may_be_named_after_a_keyword() {
    let src = "## A Memo has:\n\
        \x20   an id, which is Int.\n\
        \x20   a title, which is Text.\n\n\
        ## Component Row (note: Memo)\n\
        View:\n\
        \x20   <li>{note's title}</li>\n\n\
        ## Component App\n\
        State:\n\
        \x20   Let one be a new Memo with id 1 and title \"hi\".\n\
        View:\n\
        \x20   <ul><Row note={one}/></ul>\n\n\
        ## Main\n\
        Mount App.\n";
    assert_eq!(
        tw_outcome(src).error,
        None,
        "`(note: Memo)` is a parameter called `note`. `Note that …` is a SENTENCE opener; a \
         parameter list is not a sentence, so the two never collide in this position:\n{src}"
    );
}

/// ★ AND MOUNTING A COMPONENT THAT NEEDS PROPS IS REFUSED BY NAME, which is what the broken
/// fixtures above were tripping over. It used to compile, run, and fail at RUN time with
/// `Undefined variable: n` — a name the author DID write, reported as though misspelled.
#[test]
fn mounting_a_component_that_needs_props_says_so() {
    let src = "## A Memo has:\n\x20   a title, which is Text.\n\n\
        ## Component Row (m: Memo)\nView:\n\x20   <li>{m's title}</li>\n\n\
        ## Main\nMount Row.\n";
    let err = tw_outcome(src)
        .error
        .expect("mounting a prop-taking component is refused");
    assert!(
        err.contains("Row") && err.contains('m'),
        "the refusal names the component and the prop: {err}"
    );
    assert!(
        !err.contains("Undefined variable"),
        "and it is not reported as a missing value — the mount can never supply one: {err}"
    );
}
