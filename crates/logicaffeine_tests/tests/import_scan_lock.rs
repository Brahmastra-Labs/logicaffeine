//! ★ LOCK — an `Import` is seen wherever a literate module puts it. ★
//!
//! A `.lg` file is LITERATE: a title, prose explaining what the module is for, then the code. The
//! dependency scanner read only the file's opening ABSTRACT — it stopped at the first blank line
//! after any prose — so an `Import` written after a second paragraph was NEVER SEEN.
//!
//! ★ AND IT FAILED SILENTLY, which is what made it expensive. An import nobody scanned is an
//! import nobody resolves: the module is not spliced, its `## A Todo has:` never reaches the type
//! registry, and the program compiles on as if the record had never been declared. The tree-walker
//! survives that (it resolves loosely and the page still runs), so the first sign is a WASM refusal
//! — "field access with no static slot" — pointing at the backend's internals, in a different
//! file from the import that was dropped, with nothing anywhere saying a module went missing.
//!
//! ★ WHERE IT WAS FOUND: `examples/fullstack_todo`. Both halves of the app say `Import Shared.`
//! after their explanatory prose, and NEITHER resolved — so the one record the client and the
//! server agree about reached neither of them. The page could not be built for a browser and the
//! server's `## Route … -> Seq of Todo` was refused as "a type the derived JSON codecs do not
//! cover", which is true of a type the compiler was never told about.
//!
//! THE RULE, and it is the one a reader would guess: an `Import` sentence is an import wherever it
//! appears BEFORE the code begins. The first `## ` header still ends the scan — past that point the
//! file is code, and an import there is a statement the parser handles.
//!
//! ★ A MARKDOWN LINK IS NOT WIDENED WITH IT, and that distinction is the whole of the fix. An
//! `Import` is a DECLARATION and means what it says wherever it sits; a `[see also](other.md)` is
//! PROSE, and a module's explanation may link to things it does not depend on. Links stay
//! abstract-only, which is a deliberate design two older tests were already pinning — my first
//! version of this widened both and broke that rule while fixing this one.

use logicaffeine_language::analysis::dependencies::scan_dependencies;

fn uris(src: &str) -> Vec<String> {
    scan_dependencies(src).into_iter().map(|d| d.uri).collect()
}

#[test]
fn an_import_in_the_opening_paragraph_is_seen() {
    // The case that always worked. It must keep working — everything below widens the scan, and a
    // widening that broke the narrow case would be a trade, not a fix.
    assert_eq!(
        uris("# App\nImport Shared.\n\n## Main\nShow 1.\n"),
        ["file:./shared"]
    );
}

#[test]
fn an_import_after_a_paragraph_of_prose_is_seen() {
    // ★ THE HEADLINE, and the exact shape of every literate module in this repo: a title, a
    // sentence saying what the file is, a blank line, then the imports.
    let src = "# App\n\nThe whole frontend: what the page knows and what it does.\n\nImport Shared.\n\n## Main\nShow 1.\n";
    assert_eq!(uris(src), ["file:./shared"]);
}

#[test]
fn an_import_after_several_paragraphs_is_seen() {
    // Real files explain themselves at length before the code starts. A scan that stopped after
    // paragraph one and a scan that stops after paragraph three are the same bug at different sizes.
    let src =
        "# App\n\nFirst paragraph.\n\nSecond paragraph, longer, with a `code span` in it.\n\n\
               Third paragraph.\n\nImport Shared.\n\n## Main\nShow 1.\n";
    assert_eq!(uris(src), ["file:./shared"]);
}

#[test]
fn imports_in_different_paragraphs_are_all_seen() {
    // Finding one and stopping would be the same failure with a nicer face on it.
    let src = "# App\n\nProse.\n\nImport Shared.\n\nMore prose about the other one.\n\nImport Colors.\n\n## Main\nShow 1.\n";
    assert_eq!(uris(src), ["file:./shared", "file:./colors"]);
}

#[test]
fn several_modules_in_one_sentence_still_read_as_several() {
    // The natural-language form, after prose — both halves of the feature at once.
    let src = "# App\n\nProse.\n\nImport Shared and Colors.\n\n## Main\nShow 1.\n";
    assert_eq!(uris(src), ["file:./shared", "file:./colors"]);
}

#[test]
fn an_explicit_path_import_after_prose_is_seen() {
    // The `from "path"` form takes the same route through the scanner and must not be the one
    // shape that stayed invisible.
    let src = "# App\n\nProse.\n\nImport Shared from \"src/shared.lg\".\n\n## Main\nShow 1.\n";
    assert_eq!(uris(src), ["file:./src/shared.lg"]);
}

#[test]
fn a_markdown_link_in_the_abstract_is_still_a_dependency() {
    // The literate link form, in the place it has always counted: the opening paragraph.
    let src = "# App\n\nBuilt on [Shared](./shared.lg).\n\n## Main\nShow 1.\n";
    assert_eq!(uris(src), ["./shared.lg"]);
}

#[test]
fn a_markdown_link_after_the_abstract_is_prose_not_a_dependency() {
    // ★ AN IMPORT AND A LINK ARE NOT THE SAME KIND OF THING, and my first version of this fix
    // treated them alike — which fixed the import bug and broke the link rule in the same stroke.
    //
    // `Import Shared.` is a DECLARATION: it says the module depends on Shared, and it means that
    // wherever the author wrote it. `[see also](other.md)` is PROSE — a module's explanation may
    // link to things it does not depend on at all, and making every one a module the loader must
    // resolve turns a cross-reference into a build failure. The abstract-only rule for links is a
    // deliberate design, and two tests older than this file were pinning it.
    let src = "# App\n\nProse.\n\nSee [Other](./other.lg) for background.\n\n## Main\nShow 1.\n";
    assert_eq!(uris(src), Vec::<String>::new());
}

#[test]
fn the_code_region_still_ends_the_scan() {
    // ★ THE LIMIT THAT MUST HOLD. Past the first `## ` the file is CODE, where an `Import` is a
    // statement the parser handles and a line beginning with the word is not necessarily one at
    // all. Scanning there would mine the program's text for dependencies.
    let src = "# App\n\nProse.\n\n## Main\nImport Shared.\nShow 1.\n";
    assert_eq!(uris(src), Vec::<String>::new());
}

#[test]
fn a_note_header_ends_the_scan_too() {
    // `## Note` is how a literate `.lg` carries prose between functions — it is still a header,
    // so the prose region has ended.
    let src = "# App\n\nProse.\n\n## Note\nImport Shared.\n\n## Main\nShow 1.\n";
    assert_eq!(uris(src), Vec::<String>::new());
}

#[test]
fn prose_that_merely_mentions_importing_is_not_a_dependency() {
    // The widened scan reads more lines, so it must not read more MEANING into them. Only a line
    // that IS the import sentence counts.
    let src = "# App\n\nYou can import things here. Importing Shared would be reasonable.\n\n## Main\nShow 1.\n";
    assert_eq!(uris(src), Vec::<String>::new());
}

#[test]
fn a_file_with_no_imports_finds_none() {
    let src = "# App\n\nProse.\n\nMore prose.\n\n## Main\nShow 1.\n";
    assert_eq!(uris(src), Vec::<String>::new());
}

#[test]
fn a_file_that_is_only_prose_still_finds_its_import() {
    // A module with no code header at all (a prose-only `.md` in the closure) must not lose its
    // imports to the missing terminator.
    let src = "# Notes\n\nProse.\n\nImport Shared.\n";
    assert_eq!(uris(src), ["file:./shared"]);
}

#[test]
fn an_indented_import_inside_prose_is_not_mined() {
    // An indented line in the prose region is a code FENCE's contents or an example, not a
    // statement — and the scanner reads the trimmed line, so this is the case that says whether
    // trimming turned an illustration into a dependency.
    let src = "# App\n\nFor example:\n\n    Import Shared.\n\n## Main\nShow 1.\n";
    assert_eq!(uris(src), Vec::<String>::new());
}
