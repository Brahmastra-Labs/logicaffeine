use super::*;
use crate::stdlib_text::{module_names, module_native_names, strip_builtin_backed_decls};
use std::collections::HashSet;
use tempfile::tempdir;

/// PIT-OF-SUCCESS LOCK — the assembled prelude is TRANSITIVELY CALL-CLOSED.
///
/// `apply_prelude` must select not only the modules a program names directly, but every
/// module their `.lg` bodies transitively call: a stdlib function may freely call another
/// module's function (statistics.lg's `fsumReal` → math.lg's `absReal`; `geometricMeanReal`
/// → math.lg's `ln`/`exp`). A single non-fixpoint pass over the *program's* references alone
/// splices the named module but drops its transitive `.lg` dependencies, which then surface as
/// `Unknown function` on EVERY tier. `builtin_backed` names (sqrt, pow, …) resolve without a
/// module and hid the class — only the `.lg`-backed transitive deps regressed.
///
/// This reconstructs the failure at the loader seam: for each module, reference every function
/// it defines, assemble, and assert the result is call-closed — every prelude name the assembly
/// *calls* is either a builtin or *defined* in the assembly. A referenced-but-undefined name is
/// exactly a dropped transitive dependency. Derived from the modules themselves, so a new module
/// or a new cross-module call is covered automatically — the fixpoint can never silently regress.
#[test]
fn assembled_prelude_is_transitively_call_closed() {
    let all_defined: HashSet<String> = PRELUDE_MODULES
        .iter()
        .flat_map(|s| module_names(s))
        .collect();
    // Only `.lg`-defined names that no builtin backs can go missing — those are the deps the
    // fixpoint is responsible for pulling. Builtins resolve regardless of module selection.
    let checkable: Vec<&String> = all_defined.iter().filter(|n| !builtin_backed(n)).collect();

    for module in PRELUDE_MODULES {
        let names = module_names(module);
        if names.is_empty() {
            continue;
        }
        // A program that references (in call form) every name THIS module defines and nothing
        // else — it forces `apply_prelude` to select this module, after which the fixpoint must
        // pull whatever this module's bodies transitively call.
        let mut program = String::from("## Main\n");
        for n in &names {
            program.push_str("    Show ");
            program.push_str(n);
            program.push_str("(x).\n");
        }
        let assembled = apply_prelude(&program);
        let defined: HashSet<String> = defined_names(&assembled).into_iter().collect();

        for callee in &checkable {
            if defined.contains(callee.as_str()) {
                continue;
            }
            assert!(
                !references(&assembled, callee),
                "prelude assembly for a program using module `{}` CALLS `{}` but never \
                     defines it — apply_prelude dropped a transitive `.lg` dependency (the \
                     module-selection fixpoint regressed to a single non-transitive pass)",
                names.first().map(String::as_str).unwrap_or("?"),
                callee
            );
        }
    }
}

#[test]
fn test_file_scheme_resolution() {
    let temp_dir = tempdir().unwrap();
    let geo_path = temp_dir.path().join("geo.md");
    fs::write(
        &geo_path,
        "## Definition\nA Point has:\n    an x, which is Int.\n",
    )
    .unwrap();

    let mut loader = Loader::new(temp_dir.path().to_path_buf());
    let result = loader.resolve(&temp_dir.path().join("main.md"), "file:./geo.md");

    assert!(result.is_ok(), "Should resolve file: scheme: {:?}", result);
    assert!(result.unwrap().content.contains("Point"));
}

#[test]
fn test_logos_std_scheme() {
    let mut loader = Loader::new(PathBuf::from("."));
    let result = loader.resolve(&PathBuf::from("main.md"), "logos:std");

    assert!(result.is_ok(), "Should resolve logos:std: {:?}", result);
}

#[test]
fn test_logos_core_scheme() {
    let mut loader = Loader::new(PathBuf::from("."));
    let result = loader.resolve(&PathBuf::from("main.md"), "logos:core");

    assert!(result.is_ok(), "Should resolve logos:core: {:?}", result);
}

#[test]
fn test_unknown_intrinsic() {
    let mut loader = Loader::new(PathBuf::from("."));
    let result = loader.resolve(&PathBuf::from("main.md"), "logos:unknown");

    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Unknown intrinsic"));
}

#[test]
fn test_caching() {
    let temp_dir = tempdir().unwrap();
    let geo_path = temp_dir.path().join("geo.md");
    fs::write(&geo_path, "content").unwrap();

    let mut loader = Loader::new(temp_dir.path().to_path_buf());

    // First load
    let _ = loader.resolve(&temp_dir.path().join("main.md"), "file:./geo.md");

    // Should be cached now
    assert!(loader.loaded_modules().len() == 1);
}

#[test]
fn test_missing_file() {
    let temp_dir = tempdir().unwrap();
    let mut loader = Loader::new(temp_dir.path().to_path_buf());

    let result = loader.resolve(&temp_dir.path().join("main.md"), "file:./nonexistent.md");

    assert!(result.is_err());
    // The loader reports a clear "not found" for a missing module (naming the
    // file it looked for), rather than a raw io error.
    assert!(result.unwrap_err().contains("could not find"));
}

// ─── Demand-import reference detection ──────────────────────────────────

/// A struct FIELD named like a stdlib function must not drag that module in. `rank:` here follows
/// the header's `has:` on the PREVIOUS line — the `:` rule is for a same-line parameter/field TYPE
/// annotation (`(x: Matrix)`), never for a name that merely begins the next line after a header
/// colon. Before this lock, `rank: Divergent Int` spliced all of `lapack.lg` (which defines `rank`)
/// into a CRDT program, and the RISC-V backend then refused the program over lapack's builtins.
#[test]
fn a_field_named_like_a_stdlib_fn_does_not_reference_it() {
    let src = "## Definition\nA Page is Shared and has:\n    rank: Divergent Int.\n## Main\n    Let mutable p be a new Page.\n    Resolve p's rank to 3.\n    Show p's rank.\n";
    assert!(
        !references(src, "rank"),
        "`rank` is a field NAME here, not a use of lapack's `rank`"
    );
    let spliced = apply_prelude(src);
    assert!(
        !spliced.contains("## To rank "),
        "lapack.lg must not be spliced for a field named `rank`:\n{spliced}"
    );
}

/// The same-line type positions still count as uses: a parameter type after `:` and a return type
/// after `->`, plus the call form and the article/keyword forms.
#[test]
fn same_line_type_and_call_positions_still_reference() {
    assert!(references(
        "## To f (x: Matrix) -> Int:\n    Return 1.\n",
        "Matrix"
    ));
    assert!(references(
        "## To f (x: Int) -> Result of Int and Text:\n    Return 1.\n",
        "Result"
    ));
    assert!(references("## Main\n    Show rank(2, 2, m).\n", "rank"));
    assert!(references(
        "## Main\n    Let m be a new Matrix.\n",
        "Matrix"
    ));
    assert!(references(
        "## Main\n    Inspect r:\n        When Ok (v):\n            Show v.\n",
        "Ok"
    ));
}

/// A keyword or colon that ENDS the previous line never licenses the first word of the next line.
#[test]
fn a_line_break_between_the_licensing_token_and_the_name_is_not_a_use() {
    assert!(!references("## A Thing has:\n    rank: Int.\n", "rank"));
    assert!(!references("## To g (x: Int) ->\n    rank\n", "rank"));
    assert!(!references("Give it to\n    rank.\n", "rank"));
}

// ─── Prelude auto-import internals ──────────────────────────────────────

#[test]
fn prelude_contains_no_note_blocks() {
    // `## Note` documentation lives in the module SOURCES (the IDE reads
    // it from `prelude_module_sources`); the runtime prelude must stay
    // lean and note-free, byte-identical to the pre-documentation join.
    assert!(
        !prelude().contains("## Note"),
        "prelude() must strip documentation notes before prepending"
    );
    for src in prelude_module_sources() {
        for name in defined_names(&strip_note_blocks(module_code(src))) {
            assert!(!name.is_empty());
        }
    }
}

#[test]
fn note_stripping_is_byte_exact_around_headers() {
    let documented = "## Note\nDoes a thing.\n\n## To f (n: Int) -> Int:\n    Return n.\n";
    let bare = "## To f (n: Int) -> Int:\n    Return n.\n";
    assert_eq!(strip_note_blocks(documented), bare);

    let between = "## To a:\n    Show 1.\n\n## Note\nDoc.\n\n## To b:\n    Show 2.\n";
    let bare_between = "## To a:\n    Show 1.\n\n## To b:\n    Show 2.\n";
    assert_eq!(strip_note_blocks(between), bare_between);
}

/// A `## Note` may span several paragraphs (a one-line SUMMARY the IDE leads with, then details):
/// the runtime prelude strips ALL of it. A note ends at the next `##` header or at a bare type
/// declaration (`A Request has:` inside a `## Definition` body, core.lg's `A Result … is either:`)
/// — never at a blank line, or the details paragraph would leak into the prelude as prose and every
/// program that splices the module would fail to parse.
#[test]
fn note_stripping_removes_every_paragraph_of_a_note() {
    let multi = "## Note\nSummary sentence.\n\nDetails paragraph one, with `code`.\n\nDetails paragraph two.\n\n## To f (n: Int) -> Int:\n    Return n.\n";
    assert_eq!(
        strip_note_blocks(multi),
        "## To f (n: Int) -> Int:\n    Return n.\n"
    );

    let before_decl = "## Definition\n\n## Note\nA request as a handler sees it.\n\nMore detail about the fields.\n\nA Request has:\n    a method, which is Text.\n";
    assert_eq!(
        strip_note_blocks(before_decl),
        "## Definition\n\nA Request has:\n    a method, which is Text.\n"
    );

    let core_style = "## Note\nThe success-or-failure type.\n\nA Result of [T] and [E] is either:\n    Ok with value (T).\n";
    assert_eq!(
        strip_note_blocks(core_style),
        "A Result of [T] and [E] is either:\n    Ok with value (T).\n"
    );

    // Prose that merely STARTS with an article + capitalized word is not a declaration.
    let prose = "## Note\nA StringBuilder accumulates text.\n\nAn HTTP response a handler returns: status and body.\n\n## To g:\n    Show 1.\n";
    assert_eq!(strip_note_blocks(prose), "## To g:\n    Show 1.\n");
}

/// A type declared on its own `##` header line is a name exactly as it is under `## Definition`:
/// `## A Response has:` defines `Response`, `## An Outcome is either:` defines `Outcome` and its
/// variants. Only a type header counts after `##` — a variant line is never a section header.
#[test]
fn a_type_declared_on_its_header_line_is_a_name() {
    let code = "## A Response has:\n    a status, which is Int.\n\n## An Outcome is either:\n    Success.\n    A Failure (message: Text).\n\n## A private Secret has:\n    a key, which is Text.\n\n## A Stray (x: Int).\n\n## Main\nShow 1.\n";
    assert_eq!(
        defined_names_with_visibility(code),
        vec![
            ("Response".to_string(), false),
            ("Outcome".to_string(), false),
            ("Success".to_string(), false),
            ("Failure".to_string(), false),
            ("Secret".to_string(), true),
        ]
    );
    let definition = "## Definition\nA Response has:\n    a status, which is Int.\n";
    assert_eq!(defined_names(definition), vec!["Response"]);
}

/// A builtin-backed type's declaration is dropped from a module whether it sits in a block or on its
/// own section header, and an enum's variant block goes with it.
#[test]
fn a_builtin_backed_declaration_on_its_header_line_is_stripped() {
    let module = "## A Map of [K] and [V] is a generic collection.\n## An Option of [T] is either:\n    A Some (value: T).\n    A None.\n\n## A Point has:\n    An x: Int.\n";
    assert_eq!(
        strip_builtin_backed_decls(module),
        "## A Point has:\n    An x: Int."
    );
}

/// A function header names its function whatever follows the name — parameters, a return type,
/// or nothing but the colon — and whichever order its `native`/`private` modifiers come in.
#[test]
fn a_function_header_names_its_function_in_every_form() {
    let read =
        |line: &str| function_header(line).map(|h| (h.name.to_string(), h.native, h.private));
    let rows: &[(&str, Option<(&str, bool, bool)>)] = &[
        ("## To worker:", Some(("worker", false, false))),
        ("## To f (n: Int) -> Int:", Some(("f", false, false))),
        ("## To f(n: Int):", Some(("f", false, false))),
        ("## To greeting -> Text:", Some(("greeting", false, false))),
        (
            "## To size of [T] (t: Tree of T) -> Int:",
            Some(("size", false, false)),
        ),
        ("## To private announce:", Some(("announce", false, true))),
        (
            "## To private helper (n: Int) -> Int:",
            Some(("helper", false, true)),
        ),
        (
            "## To native args -> Seq of Text",
            Some(("args", true, false)),
        ),
        ("## To native private clock:", Some(("clock", true, true))),
        ("## To private native clock:", Some(("clock", true, true))),
        ("   ## To indented:", Some(("indented", false, false))),
        ("## To :", None),
        ("## Main", None),
        ("## A Point has:", None),
        ("To worker:", None),
    ];
    for (line, want) in rows {
        let want = want.map(|(n, native, private)| (n.to_string(), native, private));
        assert_eq!(read(line), want, "`{line}`");
    }
}

/// A variant's article is optional, so every indented line of an `is either:` block names a
/// variant, with or without one — and a function body's indented lines name nothing.
#[test]
fn a_variant_is_a_name_with_or_without_its_article() {
    let code = "## Definition\nAn Outcome is either:\n    Success (value: Int).\n    Failure with message Text.\n    Pending.\n    A Retry (after: Int).\n\n## To f:\n    Show x.\n    Report (1).\n";
    assert_eq!(
        defined_names(code),
        vec!["Outcome", "Success", "Failure", "Pending", "Retry", "f"]
    );
}

#[test]
fn derives_defined_names_per_module() {
    // Enum VARIANTS are names too (since the core.lg prelude landed):
    // `a new Ok with value …` / `When Some (v):` must pull in the module
    // that declares the enum. Common-word variants (Info, Failure) stay
    // safe through the same declarer-wins rule as everything else — a
    // user's own `Failure` enum suppresses the module entirely.
    assert_eq!(
        module_names(STD_CORE),
        vec!["Result", "Ok", "Err", "Option", "Some", "None", "List", "Map"]
    );
    assert_eq!(module_names(STD_NET), vec!["Message"]);
    assert_eq!(module_names(STD_CRDT), vec!["Delta"]);
    assert_eq!(
        module_names(STD_IO),
        vec!["Severity", "Debug", "Info", "Warning", "Failure"]
    );
    assert_eq!(module_names(STD_CONCURRENCY), vec!["flush"]);
    assert_eq!(module_names(STD_ENV), vec!["get", "args"]);
    assert_eq!(module_names(STD_FILE), vec!["read", "write"]);
    assert_eq!(module_names(STD_RANDOM), vec!["randomInt", "randomFloat"]);
    assert_eq!(module_names(STD_TIME), vec!["now", "sleep"]);
    assert_eq!(
        module_names(STD_HTTP),
        vec![
            "Request",
            // What went wrong with a request a PAGE sent — the error half of a typed
            // fetch's `Result of T and HttpError`.
            "HttpError",
            "Response",
            "plain",
            "html",
            "json",
            "notFound",
            // The two refusals a middleware guard gives (401 "who are you" vs 403 "no"), the
            // redirects, and the cookie vocabulary a session rides on. Added deliberately
            // here, which is what this list exists for.
            "unauthorized",
            "forbidden",
            "redirectTo",
            "permanentRedirectTo",
            "cookie",
            // The form that says for itself whether it is `Secure`, and the answer `cookie`
            // gives it. A `Secure` cookie never comes back over plain HTTP, so a session
            // could not survive one hop on a LAN address without this.
            "cookieFor",
            "cookiesAreSecure",
            "expiredCookie",
            "isCookieSafe",
            "withCookie",
            "cookieOf",
            "bytesResponse",
            "headerOf",
            // The same case-insensitive lookup against a header MAP rather than a request,
            // so a RESPONSE's headers are read by the one walk a request's are — which is
            // how the router finds the `Set-Cookie` a route set.
            "headerIn",
            "hasHeader",
            "queryOf",
            "hasQuery",
            "isHeaderTokenByte",
            "isHeaderName",
            "isHeaderValue",
            "isHopByHop",
            "withHeader",
        ]
    );
    // Builtin-backed collection names never TRIGGER an import (the engine
    // resolves them preludeless) — but they exist as names above so a user
    // `Map` type still suppresses core.lg, declarer-wins.
    assert!(builtin_backed("List") && builtin_backed("Map"));
    assert!(!builtin_backed("Result") && !builtin_backed("Ok"));
}

#[test]
fn references_matches_type_and_call_positions() {
    assert!(references(
        "Let m be a new Message with sender 1.",
        "Message"
    ));
    assert!(references("## To rank (s: Severity) -> Int:", "Severity"));
    assert!(references("Let xs be args().", "args"));
    assert!(references("Call flush with xs and ch.", "flush"));
    assert!(references("Launch a task to flush.", "flush"));
}

#[test]
fn references_ignores_prose_and_substrings() {
    // A bare mention in a string is not a use position.
    assert!(!references("Show \"Message received\".", "Message"));
    // Part of a larger identifier is not a whole-word match.
    assert!(!references("Let MessageBox be 1.", "Message"));
    assert!(!references("Let nowhere be 1.", "now"));
}

fn renames(pairs: &[&str]) -> std::collections::HashMap<String, String> {
    pairs
        .iter()
        .map(|n| (n.to_string(), hygienic_name(n)))
        .collect()
}

#[test]
fn hygiene_renames_a_function_at_its_header_and_calls_only() {
    let code = "## To trim (s: Text) -> Text:\n    Return s.\n\n\
                    ## To words (s: Text) -> Seq of Text:\n    Let trim be 1.\n    Let t be trim(s).\n    Call trim with s.\n    Launch a task to trim.\n    Show p's trim.\n    Return split(t, \" \").\n";
    let out = rename_for_hygiene(code, &renames(&["trim"]));
    assert!(out.contains("## To __std_trim (s: Text) -> Text:"), "{out}");
    assert!(out.contains("Let t be __std_trim(s)."), "{out}");
    assert!(out.contains("Call __std_trim with s."), "{out}");
    assert!(out.contains("a task to __std_trim."), "{out}");
    assert!(
        out.contains("Let trim be 1."),
        "a local is a different entity: {out}"
    );
    assert!(
        out.contains("p's trim."),
        "a field is a different entity: {out}"
    );
}

#[test]
fn hygiene_renames_a_header_without_parameters() {
    let out = rename_for_hygiene(
        "## To private now -> Int:\n    Return 1.\n",
        &renames(&["now"]),
    );
    assert_eq!(out, "## To private __std_now -> Int:\n    Return 1.\n");
}

#[test]
fn hygiene_leaves_strings_and_char_literals_but_renames_interpolation_holes() {
    let code = "Show \"trim(x) {{trim(x)}} {trim(x)}\".\nLet c be `\"`.\nLet d be trim(c).\n";
    let out = rename_for_hygiene(code, &renames(&["trim"]));
    assert_eq!(
        out,
        "Show \"trim(x) {{trim(x)}} {__std_trim(x)}\".\nLet c be `\"`.\nLet d be __std_trim(c).\n"
    );
}

#[test]
fn hygiene_renames_a_type_and_its_variants_everywhere_in_code() {
    let code = "## Definition\nA Level is either:\n    A Debug.\n    A Warn (code: Int).\n\n\
                    ## To f (l: Level) -> Level:\n    Inspect l:\n        When Debug:\n            Return a new Warn with code 1.\n    Show \"Debug\".\n    Return l.\n";
    let out = rename_for_hygiene(code, &renames(&["Level", "Debug", "Warn"]));
    assert!(out.contains("A Std__Level is either:"), "{out}");
    assert!(out.contains("A Std__Debug."), "{out}");
    assert!(out.contains("(l: Std__Level) -> Std__Level:"), "{out}");
    assert!(out.contains("When Std__Debug:"), "{out}");
    assert!(out.contains("a new Std__Warn with code 1."), "{out}");
    assert!(
        out.contains("Show \"Debug\"."),
        "a string is never code: {out}"
    );
}

#[test]
fn hygiene_never_touches_a_larger_identifier() {
    let code = "Let x be trimmed(s) + untrim(s) + trim_all(s).\n";
    assert_eq!(rename_for_hygiene(code, &renames(&["trim"])), code);
}

#[test]
fn local_bindings_split_callables_from_values() {
    let b = local_bindings(
        "## To f (lines: Seq of Text, n: Int) -> Int:\n    Return n.\n\
             ## Main\nLet clamp be (n: Int) -> n.\nLet mutable words be split(s, \" \").\n\
             Repeat for line in lines:\n    Show line.\nFor each c in s:\n    Show c.\n\
             Let g be (a: Int, b: Int) ->:\n    Return a.\n",
    );
    assert_eq!(b.callables, vec!["clamp", "g"]);
    for v in ["lines", "n", "words", "line", "c", "a", "b"] {
        assert!(
            b.values.iter().any(|x| x == v),
            "{v} is a value binding: {:?}",
            b.values
        );
    }
    assert!(!b.values.iter().any(|x| x == "clamp" || x == "g"));
}

#[test]
fn library_ownership_is_decided_by_content_not_spelling() {
    let spliced = apply_prelude("## Main\nShow indexOf(\"hello\", \"l\").\n").into_owned();
    let owned = library_owned_functions(&spliced);
    assert!(
        owned.contains("indexOf"),
        "the spliced library indexOf is the library's"
    );
    assert!(
        owned.contains("upper"),
        "every untouched library function is the library's"
    );

    let own = "## To indexOf (hay: Text, needle: Text) -> Int:\n    Return 42.\n\n## Main\nShow upper(indexOf(\"a\", \"b\")).\n";
    let owned = library_owned_functions(&apply_prelude(own));
    assert!(
        !owned.contains("indexOf"),
        "a program's own indexOf is never the library's"
    );
    assert!(
        !owned.contains("__std_indexOf"),
        "a renamed library copy is not under its public name"
    );
    assert!(owned.contains("upper"));

    assert!(library_owned_functions("## Main\nShow 1.\n").is_empty());
}

#[test]
fn library_ownership_survives_the_pruned_assembly_the_rust_tier_compiles() {
    let program = "## Main\nLet n be length of args() * 4096.\nLet mutable acc be 0.\nLet mutable i be 0.\n\
                       While i is less than n:\n    Set acc to acc + trunc(sin(toReal(i)) * 1000.0).\n    Set i to i + 1.\nShow acc.\n";
    let spliced = apply_prelude(program).into_owned();
    let pruned = prune_unreachable_prelude_from(program, &spliced);
    for assembled in [&spliced, &pruned] {
        let owned = library_owned_functions(assembled);
        for name in ["trunc", "toReal"] {
            assert!(
                owned.contains(name),
                "`{name}` must be the library's in:\n{assembled}"
            );
        }
    }
}

#[test]
fn module_natives_are_read_from_headers() {
    assert_eq!(
        module_native_names(module_code(STD_FILE)),
        vec!["read", "write"]
    );
    assert!(module_native_names(module_code(STD_MATH)).is_empty());
}

#[test]
fn defines_detects_native_decl_and_type() {
    assert!(defines(
        "## To native args -> Seq of Text\n## Main\n    Show 1.",
        "args"
    ));
    assert!(defines(
        "## Definition\nA Message has:\n    a kind, which is Int.",
        "Message"
    ));
    assert!(!defines("## Main\n    Let x be 1.", "Message"));
}

/// The segment contract deep check's span translation stands on: every
/// user-attributed run is byte-identical to the user's own text at its
/// recorded offset, the runs are ordered and gap-free over the output,
/// and concatenating them (plus spliced runs) reconstructs the output.
fn assert_segments_faithful(source: &str) {
    let (out, segments) = apply_prelude_mapped(source);
    let out: &str = out.as_ref();
    let mut cursor = 0usize;
    for seg in &segments {
        assert_eq!(
            seg.out_start, cursor,
            "segments must tile the output in order"
        );
        cursor += seg.len;
        if let Some(us) = seg.user_start {
            assert_eq!(
                &out[seg.out_start..seg.out_start + seg.len],
                &source[us..us + seg.len],
                "a user segment must be the user's own bytes"
            );
        }
    }
    assert_eq!(cursor, out.len(), "segments must cover the whole output");
}

#[test]
fn prelude_segments_cover_a_plain_prepend() {
    // `##` first line → the modules genuinely prepend (head is empty).
    let src = "## Main\nShow upper(\"hello\").\n";
    assert_segments_faithful(src);
    let (out, segments) = apply_prelude_mapped(src);
    assert!(out.len() > src.len(), "strings.lg must have been spliced");
    assert_eq!(
        segments.first().map(|s| s.user_start),
        Some(None),
        "splice comes first"
    );
    assert_eq!(
        segments.last().map(|s| s.user_start),
        Some(Some(0)),
        "user text follows whole"
    );
}

#[test]
fn prelude_segments_cover_a_mid_splice_after_a_title_head() {
    let src = "# My App\n\n## Main\nShow upper(\"hello\").\n";
    assert_segments_faithful(src);
    let (out, segments) = apply_prelude_mapped(src);
    assert!(out.len() > src.len(), "strings.lg must have been spliced");
    assert_eq!(segments.len(), 3, "head + splice + tail");
    assert_eq!(segments[0].user_start, Some(0), "the title head stays put");
    assert_eq!(segments[0].len, "# My App\n\n".len());
    assert_eq!(
        segments[1].user_start, None,
        "the splice sits after the head"
    );
    assert_eq!(
        segments[2].user_start,
        Some("# My App\n\n".len()),
        "tail keeps its offset"
    );
}

#[test]
fn prelude_segments_cover_crlf_sources() {
    assert_segments_faithful("# My App\r\n\r\n## Main\r\nShow upper(\"x\").\r\n");
}

#[test]
fn prelude_segments_cover_the_untouched_case() {
    let src = "## Main\nShow 1 plus 1.\n";
    assert_segments_faithful(src);
    let (out, segments) = apply_prelude_mapped(src);
    assert_eq!(
        out.as_ref(),
        src,
        "no stdlib reference → returned unchanged"
    );
    assert_eq!(
        segments,
        vec![PreludeSegment {
            out_start: 0,
            len: src.len(),
            user_start: Some(0)
        }]
    );
}

#[test]
fn no_prelude_strip_is_byte_preserving_and_mapped() {
    let src = "## NoPrelude\n## Main\nShow 1 plus 1.\n";
    assert_segments_faithful(src);
    let (out, segments) = apply_prelude_mapped(src);
    assert_eq!(
        out.as_ref(),
        "## Main\nShow 1 plus 1.\n",
        "only the marker line goes"
    );
    assert_eq!(
        segments,
        vec![PreludeSegment {
            out_start: 0,
            len: "## Main\nShow 1 plus 1.\n".len(),
            user_start: Some("## NoPrelude\n".len()),
        }]
    );
}

#[test]
fn no_prelude_strip_keeps_crlf_and_marker_mid_file() {
    let src = "## Main\r\nShow 1 plus 1.\r\n## NoPrelude\r\nShow 2 plus 2.\r\n";
    assert_segments_faithful(src);
    let (out, segments) = apply_prelude_mapped(src);
    assert_eq!(
        out.as_ref(),
        "## Main\r\nShow 1 plus 1.\r\nShow 2 plus 2.\r\n",
        "kept lines keep their CRLF bytes"
    );
    assert_eq!(segments.len(), 2, "one run each side of the removed marker");
}

/// A `## Store … shared on` whose field converges as a CRDT (`Let likes: Tally be 0.`) draws its
/// page's replica id through `randomInt` — a call the lowering writes, which the author's tokens never
/// name. The demand keys on that header and that annotation: the module is spliced and the pruner
/// keeps the function.
#[test]
fn a_shared_store_with_a_convergent_field_demands_random_int() {
    for ty in [
        "Tally",
        "ConvergentCount",
        "PNCounter",
        "GCounter",
        "SharedSet of Text",
        "ORSet of Int",
        "SharedSequence of Text",
        "RGA of Bool",
    ] {
        for binding in ["Let", "Let mutable"] {
            let src = format!(
                "## Store Board shared on \"board\"\nState:\n    Let n be 0.\n    {binding} field: {ty} be 0.\n\n## Main\nShow 1.\n"
            );
            let spliced = apply_prelude(&src).into_owned();
            assert!(
                spliced.contains("## To native randomInt"),
                "{binding} … {ty}: random.lg must be spliced:\n{spliced}"
            );
            assert!(
                prune_unreachable_prelude_from(&src, &spliced).contains("## To native randomInt"),
                "{binding} … {ty}: the pruner must keep randomInt"
            );
        }
    }
}

/// Only a converging field of a SHARED store draws a replica id: a shared store of plain values, a
/// store that is not shared, and a convergent field outside any store demand nothing.
#[test]
fn only_a_convergent_field_of_a_shared_store_demands_random_int() {
    for src in [
        "## Store Board shared on \"board\"\nState:\n    Let n be 0.\n    Let title: Text be \"x\".\n\n## Main\nShow 1.\n",
        "## Store Board\nState:\n    Let likes: Tally be 0.\n\n## Main\nShow 1.\n",
        "## Store Board persisted as \"board\"\nState:\n    Let likes: Tally be 0.\n\n## Main\nShow 1.\n",
        "## A Poll is Shared and has:\n    A votes: Tally.\n\n## Main\nShow 1.\n",
        "## Store Board shared on \"board\"\nState:\n    Let n be 0.\n\n## Component App\nState:\n    Let likes: Tally be 0.\n\n## Main\nShow 1.\n",
    ] {
        assert!(!apply_prelude(src).contains("## To native randomInt"), "no replica id to draw:\n{src}");
    }
}

/// The modules the build script embeds are the files `PRELUDE_FILES` names, in its order, and every table it writes
/// is what `stdlib_text::index` derives from them: a stale or misread index would splice, rename and prune by
/// another library than the one shipped.
#[test]
fn the_embedded_stdlib_index_is_the_one_its_modules_derive() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/std");
    assert_eq!(
        PRELUDE_MODULES.len(),
        crate::stdlib_text::PRELUDE_FILES.len()
    );
    for (source, stem) in PRELUDE_MODULES
        .iter()
        .zip(crate::stdlib_text::PRELUDE_FILES)
    {
        let on_disk = std::fs::read_to_string(dir.join(format!("{stem}.lg"))).unwrap();
        assert!(*source == on_disk, "{stem}.lg is embedded as another text");
    }
    let ix = crate::stdlib_text::index(&PRELUDE_MODULES);
    let owned = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<_>>();
    for m in 0..PRELUDE_MODULES.len() {
        let stem = crate::stdlib_text::PRELUDE_FILES[m];
        assert_eq!(owned(PRELUDE_NAMES[m]), ix.names[m], "{stem}: names");
        assert_eq!(
            owned(PRELUDE_NAMES[m]),
            module_names(PRELUDE_MODULES[m]),
            "{stem}: names"
        );
        assert_eq!(owned(PRELUDE_NATIVES[m]), ix.natives[m], "{stem}: natives");
        let pulls: Vec<usize> = PRELUDE_PULLS[m].iter().map(|&k| k as usize).collect();
        assert_eq!(pulls, ix.pulls[m], "{stem}: pulls");
        let code = crate::stdlib_text::spliced_code(PRELUDE_MODULES[m]);
        let scanned: Vec<usize> = (0..PRELUDE_MODULES.len())
            .filter(|&k| {
                k != m
                    && PRELUDE_NAMES[k]
                        .iter()
                        .any(|n| !builtin_backed(n) && references(&code, n))
            })
            .collect();
        assert_eq!(pulls, scanned, "{stem}: pulls, scanned name by name");
        assert_eq!(
            PRELUDE_MENTIONS_TEXT_FROM_BYTES[m],
            code.contains("text_from_bytes"),
            "{stem}: text_from_bytes"
        );
        let splice: Vec<(usize, usize)> = PRELUDE_SPLICE[m]
            .iter()
            .map(|&(s, e)| (s as usize, e as usize))
            .collect();
        assert_eq!(splice, ix.splice[m], "{stem}: splice");
        assert!(
            spliced_module(m) == code,
            "{stem}: the splice rebuilds another text"
        );
    }
    assert_eq!(owned(&PRELUDE_FUNCTIONS), ix.functions);
    let library: Vec<(String, usize, usize, usize)> = LIBRARY_SECTIONS
        .iter()
        .map(|&(n, m, s, e)| (n.to_string(), m as usize, s as usize, e as usize))
        .collect();
    assert_eq!(library, ix.library);
    for (name, text) in PRELUDE_MODULES
        .iter()
        .map(|source| crate::stdlib_text::spliced_code(source))
        .flat_map(|code| {
            function_section_ranges(&code)
                .into_iter()
                .map(|(name, range)| (name, code[range].to_string()))
                .collect::<Vec<_>>()
        })
        .collect::<HashMap<String, String>>()
    {
        assert_eq!(
            library_section(&name),
            Some(text.as_str()),
            "`## To {name}`"
        );
    }
}

/// Every program in the repository and every stdlib module, against every prelude name: a name that is not one of
/// a text's whole words is found by none of the scans the demand check runs.
#[test]
fn the_whole_word_prefilter_hides_no_use() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut texts: Vec<String> = PRELUDE_MODULES.iter().map(|s| s.to_string()).collect();
    let mut dirs = vec![root.join("examples"), root.join("benchmarks/programs")];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "lg") {
                texts.push(std::fs::read_to_string(&path).unwrap());
            }
        }
    }
    assert!(
        texts.len() > 100,
        "the corpus went missing: {} texts",
        texts.len()
    );
    for text in &texts {
        let words = whole_words(text);
        for name in PRELUDE_NAMES.iter().flat_map(|names| names.iter()) {
            if !may_name(&words, name) {
                assert!(
                    !references(text, name)
                        && !references_call_only(text, name)
                        && !references_as_filter(text, name),
                    "`{name}` hides a use"
                );
            }
        }
    }
}

/// A native is bound to its runtime implementation by name on every tier, so hygiene can never rename one: a program
/// that declares one of a module's natives itself keeps that whole module out, even while it calls the module's other
/// functions, rather than splice a library copy whose native answers to a name no runtime binds.
#[test]
fn a_program_declaring_a_modules_native_keeps_the_module_out() {
    let mut exercised = 0;
    for m in 0..PRELUDE_MODULES.len() {
        let others: Vec<&str> = PRELUDE_NAMES[m]
            .iter()
            .filter(|n| !PRELUDE_NATIVES[m].contains(n))
            .filter(|n| n.starts_with(|c: char| c.is_ascii_lowercase()) && !builtin_backed(n))
            .copied()
            .collect();
        if others.is_empty() {
            continue;
        }
        let calls: String = others
            .iter()
            .map(|n| format!("    Show {n}(1).\n"))
            .collect();
        for native in PRELUDE_NATIVES[m] {
            let header = PRELUDE_MODULES[m]
                .lines()
                .find(|l| function_header(l).is_some_and(|h| h.name == *native && h.native))
                .unwrap();
            let src = format!("{header}\n\n## Main\n{calls}");
            let spliced = apply_prelude(&src);
            let renamed = format!("## To native {}", hygienic_name(native));
            assert!(
                !spliced.contains(&renamed),
                "`{native}` was renamed:\n{spliced}"
            );
            assert!(
                !spliced.contains(&format!("## To {}", others[0])),
                "the module declaring `{native}` was spliced beside the program's own `{native}`"
            );
            exercised += 1;
        }
    }
    assert!(
        exercised > 0,
        "no module has both a native and another function"
    );
}

/// The WebAssembly tier validates the bytes `text_from_bytes` is handed by calling `strings.lg`'s `validUtf8`, so a
/// program that reaches the builtin only through a library function still has the validator spliced: the demand is
/// keyed on the selected modules as well as on the program's own text.
#[test]
fn a_module_that_calls_text_from_bytes_brings_the_validator() {
    let mut exercised = 0;
    for m in (0..PRELUDE_MODULES.len()).filter(|&m| PRELUDE_MENTIONS_TEXT_FROM_BYTES[m]) {
        let Some(f) = PRELUDE_NAMES[m].iter().find(|n| {
            n.starts_with(|c: char| c.is_ascii_lowercase())
                && !builtin_backed(n)
                && !PRELUDE_NATIVES[m].contains(n)
        }) else {
            continue;
        };
        let src = format!("## Main\n    Show {f}(1).\n");
        let spliced = apply_prelude(&src);
        assert!(
            spliced.contains("## To validUtf8"),
            "a program calling `{f}` lacks the validator"
        );
        exercised += 1;
    }
    assert!(exercised > 0, "no module calls text_from_bytes");
    assert!(
        apply_prelude("## Main\n    Show fromHex(\"ff\").\n").contains("## To validUtf8"),
        "fromHex decodes straight into text_from_bytes"
    );
}

/// A real number a program defines by a function reads the function only through the phrase that names it, so
/// the tree-shaking ahead of the C backends keeps a function that phrase names, and drops one only prose names.
#[test]
fn a_function_a_defined_real_names_is_reachable() {
    let functions = "## To seventh (i: Int) -> Int:\n    Return i % 7.\n\n## To rootTwo (n: Int) -> Rational:\n    Return 1.\n\n## To unused (n: Int) -> Int:\n    Return n.\n\n";
    let source = format!(
        "{functions}## Main\nLet r be the real number whose digits in base 10 are seventh.\nLet s be the real number approximated by rootTwo.\nShow \"a phrase: the real number approximated by unused\".\n"
    );
    let kept = keep_reachable_functions(&source, &source);
    assert!(kept.contains("## To seventh"), "{kept}");
    assert!(kept.contains("## To rootTwo"), "{kept}");
    assert!(!kept.contains("## To unused"), "{kept}");
}
