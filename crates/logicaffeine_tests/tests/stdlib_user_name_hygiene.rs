//! Stdlib name hygiene: a program's own names never cost it the standard library.
//!
//! The prelude auto-imports a module when a program uses its vocabulary, and the program's own
//! definition of a name always wins ("declarer wins"). That rule must hold PER NAME, not per
//! module:
//!
//! * A plain local variable that happens to share a stdlib function's name (`Let words be …`,
//!   `Let lines be …`, `Let pi be …`, `Let title be …`) is a variable, not a definition. Values and
//!   functions live side by side — `words` reads the variable, `words(s)` calls the function — so
//!   such a binding must neither suppress the module nor be hijacked by it.
//! * A program that defines its own `## To trim` gets ITS `trim` at every call it writes, and still
//!   gets the rest of `strings.lg` (`upper`, `split`, …). Stdlib functions that call `trim`
//!   internally keep calling the stdlib's `trim`: the module is spliced with its colliding
//!   definition renamed out of the program's way (hygiene), never with the program's function
//!   silently substituted into library code.
//! * A local CLOSURE sharing a stdlib function's name is a callable the program wrote; calls to it
//!   reach the closure, never the library function.
//!
//! The pre-hygiene loader vetoed a module wholesale when the program bound ANY of its names, so the
//! single most natural line in text processing — `Let words be split(s, " ").` — failed with
//! `Unknown function: split`.

mod common;
use common::{assert_all_tiers, assert_compiled_equals_interpreted, compile_to_rust};
use logicaffeine_compile::loader::apply_prelude;

// ─── Plain locals never cost the program a module ────────────────────────────

#[test]
fn a_local_named_words_keeps_split() {
    assert_all_tiers(
        "## Main\n\
         Let s be \"hello big world\".\n\
         Let words be split(s, \" \").\n\
         Show length of words.\n\
         Show item 2 of words.\n",
        "3\nbig",
    );
}

#[test]
fn a_local_named_lines_keeps_split_and_upper() {
    assert_all_tiers(
        "## Main\n\
         Let lines be split(\"a,b,c\", \",\").\n\
         Repeat for line in lines:\n\
         \x20   Show upper(line).\n",
        "A\nB\nC",
    );
}

#[test]
fn a_local_named_pi_keeps_math() {
    assert_all_tiers(
        "## Main\n\
         Let pi be 3.\n\
         Show gcd(12, 18) + pi.\n",
        "9",
    );
}

#[test]
fn a_local_named_title_keeps_strings() {
    assert_all_tiers(
        "## Main\n\
         Let title be \"the old man\".\n\
         Show upper(title).\n",
        "THE OLD MAN",
    );
}

#[test]
fn a_mutable_local_named_like_a_stdlib_fn_keeps_the_module() {
    assert_all_tiers(
        "## Main\n\
         Let mutable sign be 1.\n\
         Set sign to sign + 1.\n\
         Show sign.\n\
         Show gcd(8, 12).\n",
        "2\n4",
    );
}

#[test]
fn a_local_and_the_stdlib_function_it_is_named_after_coexist() {
    // `Let words be words(s).` — the call reaches the stdlib function, the name then holds the
    // value. Both uses stay available afterwards.
    assert_all_tiers(
        "## Main\n\
         Let words be words(\"one two three\").\n\
         Show length of words.\n\
         Show length of words(\"x y\").\n",
        "3\n2",
    );
}

#[test]
fn a_local_named_words_keeps_split_on_the_rust_tier() {
    assert_compiled_equals_interpreted(
        "## Main\n\
         Let words be split(\"hello big world\", \" \").\n\
         Show length of words.\n\
         Let pi be 3.\n\
         Show gcd(12, 18) + pi.\n",
    );
}

// ─── A program's own function wins, and the rest of the module stays ─────────

#[test]
fn a_user_trim_wins_and_the_rest_of_strings_stays() {
    assert_all_tiers(
        "## To trim (s: Text) -> Text:\n\
         \x20   Return \"mine\".\n\
         \n\
         ## Main\n\
         Show trim(\"  x  \").\n\
         Show upper(\"abc\").\n\
         Show length of split(\"a b\", \" \").\n",
        "mine\nABC\n2",
    );
}

#[test]
fn library_code_keeps_calling_the_library_function_the_program_overrode() {
    // strings.lg's `equalFold` compares through `lower`, `splitN(…, 0)` delegates to `split`, and
    // forms.lg's `isBlank` measures `trim` — a DIFFERENT module. The program's own `lower`/`split`/
    // `trim` must not leak into any of them.
    assert_all_tiers(
        "## To lower (s: Text) -> Text:\n\
         \x20   Return \"MINE\".\n\
         \n\
         ## To split (s: Text, sep: Text) -> Seq of Text:\n\
         \x20   Return [\"mine\"].\n\
         \n\
         ## To trim (s: Text) -> Text:\n\
         \x20   Return \"never blank\".\n\
         \n\
         ## Main\n\
         Show lower(\"Q\").\n\
         Show equalFold(\"AbC\", \"aBc\").\n\
         Show length of splitN(\"a,b,c\", \",\", 0).\n\
         Show length of split(\"a,b,c\", \",\").\n\
         Show isBlank(\"   \").\n\
         Show trim(\"  \").\n",
        "MINE\ntrue\n3\n1\ntrue\nnever blank",
    );
}

#[test]
fn a_user_gcd_wins_and_the_rest_of_math_stays() {
    assert_all_tiers(
        "## To gcd (a: Int, b: Int) -> Int:\n\
         \x20   Return 0 - 1.\n\
         \n\
         ## Main\n\
         Show gcd(12, 18).\n\
         Show lcm(4, 6).\n\
         Show factorial(5).\n",
        "-1\n12\n120",
    );
}

#[test]
fn a_user_function_override_holds_on_the_rust_tier() {
    assert_compiled_equals_interpreted(
        "## To lower (s: Text) -> Text:\n\
         \x20   Return \"MINE\".\n\
         \n\
         ## Main\n\
         Show lower(\"Q\").\n\
         Show equalFold(\"AbC\", \"aBc\").\n\
         Show upper(\"abc\").\n",
    );
}

#[test]
fn a_user_function_sharing_an_intrinsic_name_is_never_replaced_on_the_rust_tier() {
    // The Rust tier lowers the LIBRARY's `indexOf`/`countSub` to memchr, its `rotateLeft`/
    // `reverseBits` at a native width to one CPU instruction, and `trunc` inside a float
    // accumulator loop to AVX2. None of that may touch a program's own function of that name.
    // Recursive bodies keep the inliner from hiding the call.
    assert_compiled_equals_interpreted(
        "## To indexOf (hay: Text, needle: Text) -> Int:\n\
         \x20   If length of hay is greater than 20:\n\
         \x20       Return 0.\n\
         \x20   Return indexOf(hay + \"x\", needle) + 1.\n\
         \n\
         ## To countSub (hay: Text, needle: Text) -> Int:\n\
         \x20   If length of hay is greater than 9:\n\
         \x20       Return length of needle.\n\
         \x20   Return countSub(hay + \"yy\", needle) * 2.\n\
         \n\
         ## To rotateLeft (x: Int, k: Int, w: Int) -> Int:\n\
         \x20   If x is greater than 50:\n\
         \x20       Return x + k + w.\n\
         \x20   Return rotateLeft(x + 7, k, w) + 1.\n\
         \n\
         ## To reverseBits (x: Int, w: Int) -> Int:\n\
         \x20   If x is greater than 40:\n\
         \x20       Return x - w.\n\
         \x20   Return reverseBits(x + 9, w) * 3.\n\
         \n\
         ## To trunc (x: Real) -> Int:\n\
         \x20   If x is greater than 1000.0:\n\
         \x20       Return 7.\n\
         \x20   Return trunc(x + 250.0) + 1.\n\
         \n\
         ## Main\n\
         Let a be \"hello\".\n\
         Show indexOf(a, \"l\").\n\
         Show countSub(a, \"ll\").\n\
         Show rotateLeft(length of a, 3, 32).\n\
         Show reverseBits(length of a, 16).\n\
         Let mutable acc be 0.\n\
         Let mutable i be 0.\n\
         While i is less than 64:\n\
         \x20   Set acc to acc + trunc(sin(toReal(i)) * 3.0).\n\
         \x20   Set i to i + 1.\n\
         Show acc.\n",
    );
}

#[test]
fn the_librarys_text_search_lowers_to_memchr_and_a_programs_own_never_does() {
    let library = compile_to_rust(
        "## Main\nLet h be \"hello\".\nShow indexOf(h, \"l\").\nShow countSub(h, \"l\").\n",
    )
    .expect("compiles");
    assert!(
        library.contains("memchr::memchr("),
        "the library's indexOf must lower to memchr:\n{library}"
    );
    assert!(
        library.contains("memmem::find_iter("),
        "the library's countSub must lower to memmem:\n{library}"
    );
    let own = compile_to_rust(
        "## To indexOf (hay: Text, needle: Text) -> Int:\n\
         \x20   If length of hay is greater than 20:\n\
         \x20       Return 0.\n\
         \x20   Return indexOf(hay + \"x\", needle) + 1.\n\
         \n\
         ## Main\nLet h be \"hello\".\nShow indexOf(h, \"l\").\n",
    )
    .expect("compiles");
    assert!(
        !own.contains("memchr::"),
        "a program's own indexOf must never become memchr:\n{own}"
    );
}

#[test]
fn the_librarys_trunc_reduction_lifts_to_avx2_and_a_programs_own_never_does() {
    let body = "## Main\n\
                Let n be length of args() * 4096.\n\
                Let mutable acc be 0.\n\
                Let mutable i be 0.\n\
                While i is less than n:\n\
                \x20   Set acc to acc + trunc(sin(toReal(i)) * 1000.0).\n\
                \x20   Set i to i + 1.\n\
                Show acc.\n";
    let library = compile_to_rust(body).expect("compiles");
    assert!(
        library.contains("__logos_simd_chunks"),
        "the library's trunc reduction must lift to AVX2:\n{library}"
    );
    let own = compile_to_rust(&format!(
        "## To trunc (x: Real) -> Int:\n\
         \x20   If x is greater than 1000.0:\n\
         \x20       Return 7.\n\
         \x20   Return trunc(x + 250.0) + 1.\n\
         \n{body}"
    ))
    .expect("compiles");
    assert!(
        !own.contains("__logos_simd_chunks"),
        "a program's own trunc must never be lifted into an AVX2 reduction:\n{own}"
    );
}

// ─── A local closure is the program's callable ───────────────────────────────

#[test]
fn a_closure_named_like_a_stdlib_fn_is_called_and_the_module_stays() {
    assert_all_tiers(
        "## Main\n\
         Let clamp be (n: Int) -> n + 1.\n\
         Show clamp(5).\n\
         Show gcd(4, 6).\n",
        "6\n2",
    );
}

#[test]
fn a_block_closure_named_like_a_stdlib_fn_is_called() {
    assert_all_tiers(
        "## Main\n\
         Let clamp be (n: Int) ->:\n\
         \x20   If n > 100:\n\
         \x20       Return 100.\n\
         \x20   Return n.\n\
         Show clamp(150).\n",
        "100",
    );
}

// ─── The loader contract: pure, idempotent, byte-identical when unused ───────

#[test]
fn a_plain_local_referenced_only_as_a_value_imports_nothing() {
    // `perm`, `rank` and `words` are stdlib functions; here they are only ever values.
    let src = "## Main\n\
               Let perm be a new Seq of Int.\n\
               Push 3 to perm.\n\
               Let rank be item 1 of perm.\n\
               Let words be length of perm.\n\
               Show rank + words.\n";
    assert!(
        matches!(apply_prelude(src), std::borrow::Cow::Borrowed(_)),
        "a program that only uses locals must pass through untouched:\n{}",
        apply_prelude(src)
    );
    assert_all_tiers(src, "4");
}

#[test]
fn a_program_defining_every_name_it_calls_imports_nothing() {
    let src = "## To trim (s: Text) -> Text:\n\
               \x20   Return s.\n\
               \n\
               ## Main\n\
               Show trim(\"x\").\n";
    assert!(
        matches!(apply_prelude(src), std::borrow::Cow::Borrowed(_)),
        "calling only its own `trim` must not splice strings.lg"
    );
}

#[test]
fn hygienic_splicing_is_idempotent() {
    for src in [
        "## To trim (s: Text) -> Text:\n    Return s.\n\n## Main\nShow upper(trim(\"x\")).\n",
        "## Main\nLet words be split(\"a b\", \" \").\nShow length of words.\n",
        "## Main\nLet clamp be (n: Int) -> n.\nShow clamp(gcd(2, 4)).\n",
    ] {
        let once = apply_prelude(src).into_owned();
        let twice = apply_prelude(&once).into_owned();
        assert_eq!(
            once, twice,
            "applying the prelude twice must be a no-op the second time"
        );
    }
}

#[test]
fn an_overridden_name_is_defined_exactly_once_under_its_own_name() {
    let src = "## To trim (s: Text) -> Text:\n    Return s.\n\n## Main\nShow upper(trim(\"x\")).\n";
    let out = apply_prelude(src).into_owned();
    let defs = out
        .lines()
        .filter(|l| l.trim_start().starts_with("## To trim "))
        .count();
    assert_eq!(
        defs, 1,
        "exactly the program's own `trim` may be defined as `trim`:\n{out}"
    );
    assert!(
        out.contains("## To upper "),
        "strings.lg must still be spliced for `upper`"
    );
}

// ─── Exhaustive: EVERY stdlib name can be the program's own ──────────────────
//
// For every function and type any prelude module defines, a program that declares the same name
// itself — while ALSO pulling that module in through a sibling function — must compile on the VM
// (which compiles every function body, so a dangling or duplicated name is a compile error), run,
// reach its OWN definition, and leave no hidden-name call without a definition. The sibling is
// called through a probe that copies the sibling's own header, so its arguments type-check by
// construction, whatever the module.

struct StdFn {
    name: String,
    params: String,
    param_names: Vec<String>,
    returns: bool,
    native: bool,
}

fn std_functions(module: &str) -> Vec<StdFn> {
    let code = match module.find("\n## ") {
        Some(i) => &module[i + 1..],
        None => module,
    };
    let mut out = Vec::new();
    for line in code.lines() {
        let Some(mut rest) = line.trim().strip_prefix("## To ") else {
            continue;
        };
        let mut native = false;
        loop {
            if let Some(r) = rest.strip_prefix("native ") {
                native = true;
                rest = r;
            } else if let Some(r) = rest.strip_prefix("private ") {
                rest = r;
            } else {
                break;
            }
        }
        let end = rest
            .find(|c: char| c == '(' || c == ':' || c.is_whitespace())
            .unwrap_or(rest.len());
        let name = rest[..end].to_string();
        let params = rest[end..].trim_end().trim_end_matches(':').to_string();
        let mut param_names = Vec::new();
        for (i, _) in params.match_indices(':') {
            let before = params[..i].trim_end();
            let start = before
                .rfind(|c: char| !(c.is_alphanumeric() || c == '_'))
                .map(|p| p + 1)
                .unwrap_or(0);
            let opener = before[..start].trim_end();
            if (opener.ends_with('(') || opener.ends_with(',')) && start < before.len() {
                param_names.push(before[start..].to_string());
            }
        }
        out.push(StdFn {
            returns: params.contains("->"),
            name,
            params,
            param_names,
            native,
        });
    }
    out
}

fn std_types(module: &str) -> Vec<String> {
    let code = match module.find("\n## ") {
        Some(i) => &module[i + 1..],
        None => module,
    };
    let mut out = Vec::new();
    let mut in_note = false;
    for line in code.lines() {
        let t = line.trim();
        if t.starts_with("## ") {
            in_note = t == "## Note";
        }
        if in_note {
            continue;
        }
        let Some(rest) = t.strip_prefix("A ").or_else(|| t.strip_prefix("An ")) else {
            continue;
        };
        let word = rest
            .split(|c: char| c == '(' || c == '.' || c.is_whitespace())
            .next()
            .unwrap_or("");
        if !word.starts_with(|c: char| c.is_ascii_uppercase()) {
            continue;
        }
        let tail = rest[word.len()..].trim_start();
        if tail.starts_with("has") || tail.starts_with("is") || tail.starts_with("of") {
            if !matches!(word, "List" | "Map" | "Option" | "Some" | "None")
                && !out.contains(&word.to_string())
            {
                out.push(word.to_string());
            }
        }
    }
    out
}

/// Whether `line` is the header of function `name`, in either form the language accepts:
/// `## To name (x: Int) …` or `## To name(x: Int) …`.
fn defines_function(line: &str, name: &str) -> bool {
    line.trim_start()
        .strip_prefix("## To ")
        .and_then(|rest| rest.strip_prefix(name))
        .is_some_and(|tail| tail.starts_with(' ') || tail.starts_with('('))
}

fn hygiene_violation(program: &str, own: &str, expected: &str) -> Option<String> {
    let expanded = apply_prelude(program).into_owned();
    let own_defs = expanded
        .lines()
        .filter(|l| defines_function(l, own) || l.trim_start().starts_with(&format!("## A {own} ")))
        .count();
    if own_defs != 1 {
        return Some(format!(
            "`{own}` is defined {own_defs} times after splicing"
        ));
    }
    for (i, _) in expanded.match_indices("__std_") {
        let tail = &expanded[i..];
        let end = tail
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(tail.len());
        let hidden = &tail[..end];
        if tail[end..].starts_with('(') && !expanded.lines().any(|l| defines_function(l, hidden)) {
            return Some(format!("`{hidden}` is called but never defined"));
        }
    }
    let outcome = logicaffeine_compile::compile::vm_outcome(program);
    if let Some(err) = outcome.error {
        return Some(format!("VM error: {err}"));
    }
    (outcome.output.trim() != expected)
        .then(|| format!("output {:?}, expected {expected:?}", outcome.output.trim()))
}

#[test]
fn every_stdlib_name_can_be_the_programs_own() {
    let modules = logicaffeine_compile::loader::prelude_module_sources();
    let mut cases: Vec<(String, String, String)> = Vec::new();
    let mut unforceable = Vec::new();
    for module in modules {
        let fns = std_functions(module);
        let mut owned: Vec<String> = fns
            .iter()
            .filter(|f| !f.native)
            .map(|f| f.name.clone())
            .collect();
        owned.extend(std_types(module));
        for own in owned {
            let Some(sibling) = fns
                .iter()
                .find(|f| f.name != own && f.returns && !f.params.contains('[') && !f.native)
            else {
                unforceable.push(own);
                continue;
            };
            let probe = format!(
                "## To hygieneProbe {}:\n    Return {}({}).\n\n",
                sibling.params,
                sibling.name,
                sibling.param_names.join(", ")
            );
            let (decl, main) = if own.starts_with(|c: char| c.is_ascii_uppercase()) {
                (
                    format!("## A {own} has:\n    a hygieneField, which is Int.\n\n"),
                    format!("## Main\nLet t be a new {own} with hygieneField 424242.\nShow t's hygieneField.\n"),
                )
            } else {
                (
                    format!("## To {own} (hygieneArg: Int) -> Int:\n    Return hygieneArg + 424241.\n\n"),
                    format!("## Main\nShow {own}(1).\n"),
                )
            };
            cases.push((own, format!("{decl}{probe}{main}"), "424242".to_string()));
        }
    }
    assert!(
        cases.len() > 1000,
        "the sweep must cover the whole stdlib, got {}",
        cases.len()
    );
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(16);
    let chunk = cases.len().div_ceil(threads);
    let failures: Vec<String> = std::thread::scope(|s| {
        let handles: Vec<_> = cases
            .chunks(chunk)
            .map(|batch| {
                s.spawn(move || {
                    batch
                        .iter()
                        .filter_map(|(own, program, expected)| {
                            hygiene_violation(program, own, expected)
                                .map(|why| format!("{own}: {why}\n--- program ---\n{program}"))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect()
    });
    assert!(
        failures.is_empty(),
        "{} of {} stdlib names cannot be a program's own:\n\n{}",
        failures.len(),
        cases.len(),
        failures
            .iter()
            .take(25)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n\n")
    );
    assert!(
        unforceable.len() < 20,
        "names with no sibling to force their module: {unforceable:?}"
    );
}
