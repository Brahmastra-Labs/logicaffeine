//! ★ LOCK — a standard-library call the COMPILER writes must declare its own demand. ★
//!
//! The prelude is loaded on demand, and the demand is computed by scanning **the author's tokens**.
//! That works for every call an author writes and fails silently for every call a LOWERING writes:
//! the function is emitted into the program, the scan never saw its name, the module is pruned, and
//! the build dies at `Unknown function: <name>` pointing at code nobody typed.
//!
//! ★ FOUR CONSTRUCTS HAVE NOW HIT THIS, and the fourth was written tonight:
//!   * `htmlEscape` — the first, and the one the loader's own comments still cite
//!   * the JSON derive's `toJson_`/`fromJson_` encoders, whose calls the router synthesizes
//!   * `__html_<Name>`, reached from a server-rendered route rather than from any source token
//!   * `safeUrl`, added with the URL-scheme guard
//!
//! Each time, the rule was rediscovered and then written down *at the site* — a comment beside the
//! fix, which is precisely where the fifth person will not be looking. The rule belongs at the seam:
//!
//!     A DEMAND SCAN READS THE AUTHOR'S TOKENS, SO ANYTHING THE COMPILER WRITES IS INVISIBLE TO IT
//!     AND MUST BE DECLARED.
//!
//! ★ THIS IS A RATCHET, NOT A BUG FIX. Every emitted call is registered today; I checked all
//! twenty-two before writing this, so it is green on arrival. Its whole value is the fifth time —
//! it fails when a lowering starts calling a standard-library function nobody registered, which is
//! the moment the mistake is cheap instead of the moment a page will not build. `wasm_gated_suite_lock`
//! and the catalogue half of `server_surface_deploys_lock` are the same shape.
//!
//! ★ AND IT CARRIES ITS OWN ANTI-VACUITY GUARD, because a green-on-arrival lock is exactly the kind
//! that can be silently blind — `the_extractor_actually_finds_calls` proves the scan is not simply
//! returning nothing, and `an_unregistered_call_would_be_caught` proves the comparison can fail.
//! Without those two this file would pass just as happily if either half were broken.

use std::collections::BTreeSet;
use std::path::PathBuf;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_compile")
}

fn read(rel: &str) -> String {
    let p: PathBuf = crate_dir().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

#[path = "support/lgx_lower_source.rs"]
mod lgx_lower_source;

/// Every name a lowering hands to the interner as a literal — `intern("safeUrl")`.
///
/// Interning a literal is how the lowering names anything it emits: a function to call, a field to
/// read, a temporary to bind. Most are not standard-library functions, and the intersection below
/// is what narrows this to the ones that are.
fn interned_names(src: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = src;
    while let Some(at) = rest.find("intern(\"") {
        rest = &rest[at + "intern(\"".len()..];
        if let Some(end) = rest.find('"') {
            let name = &rest[..end];
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                out.insert(name.to_string());
            }
        }
    }
    out
}

/// Every `## To <name>` the standard library defines.
fn stdlib_functions() -> BTreeSet<String> {
    let dir: PathBuf = crate_dir().join("assets/std");
    let mut out = BTreeSet::new();
    for entry in std::fs::read_dir(&dir).expect("assets/std").flatten() {
        let path: PathBuf = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("lg") {
            continue;
        }
        for line in std::fs::read_to_string(&path).unwrap_or_default().lines() {
            let Some(rest) = line.strip_prefix("## To ") else {
                continue;
            };
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                out.insert(name);
            }
        }
    }
    out
}

/// The standard-library functions the LGX lowering (the `parser/lgx_lower/` module) emits calls to —
/// the lowering that turns author syntax into ordinary LOGOS, where an emitted call comes from.
fn emitted_stdlib_calls() -> BTreeSet<String> {
    let stdlib = stdlib_functions();
    let mut out = BTreeSet::new();
    for (_, source) in lgx_lower_source::files() {
        for name in interned_names(&source) {
            if stdlib.contains(&name) {
                out.insert(name);
            }
        }
    }
    out
}

/// Whether the loader declares a demand for `name` — a virtual ref, a DCE root, or both.
///
/// ★ COMMENTS ARE STRIPPED FIRST, and I learned that the hard way while proving this lock works.
/// My first attempt to make it fail commented the two `safeUrl` registrations out — and the test
/// still passed, because `// virtual_refs.push("safeUrl");` still *contains* the quoted name. A
/// commented-out registration is exactly the state this lock exists to catch (someone disabling a
/// line to chase something else and forgetting), so it has to look at code rather than at text.
///
/// The name is matched QUOTED. Prose about a function mentions it in backticks, and the loader's
/// comments are full of `htmlEscape` — matching bare would let a doc comment satisfy the demand.
fn loader_declares(name: &str) -> bool {
    let src = read("src/loader.rs");
    let code: String = src
        .lines()
        .map(|l| match l.find("//") {
            Some(at) => &l[..at],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n");
    code.contains(&format!("\"{name}\""))
}

#[test]
fn every_emitted_stdlib_call_declares_its_demand() {
    let missing: Vec<String> = emitted_stdlib_calls()
        .into_iter()
        .filter(|n| !loader_declares(n))
        .collect();
    assert!(
        missing.is_empty(),
        "a lowering emits a call to {missing:?}, and the loader declares no demand for it.\n\n\
         The prelude is pruned by scanning the AUTHOR'S tokens, so a call the COMPILER writes is \
         invisible to that scan — the module gets pruned and the build dies at `Unknown function`, \
         pointing at code nobody typed. Register it in `loader.rs` the way `matchRoute` is: a \
         `virtual_refs.push(..)` under the condition that reaches this lowering, and a matching \
         DCE root."
    );
}

/// The BUILTINS the loader keys a standard-library demand on by finding their name in the author's
/// source — `source.contains("text_from_bytes")` demands `validUtf8` — read from `loader.rs`'s code.
fn builtins_whose_demand_the_loader_reads_from_the_source() -> BTreeSet<String> {
    let src = read("src/loader.rs");
    let code: String = src
        .lines()
        .map(|l| match l.find("//") {
            Some(at) => &l[..at],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut out = BTreeSet::new();
    let mut rest = code.as_str();
    while let Some(at) = rest.find("source.contains(\"") {
        rest = &rest[at + "source.contains(\"".len()..];
        let Some(end) = rest.find('"') else { break };
        let name = &rest[..end];
        if logicaffeine_compile::semantics::builtins::builtin_from_name(name).is_some() {
            out.insert(name.to_string());
        }
    }
    out
}

/// ★ THE TRANSITIVE HALF. A builtin can carry a demand of its own: `text_from_bytes` validates
/// through `strings.lg`'s `validUtf8`, and the loader finds that edge by reading the AUTHOR's text
/// for the builtin's name. A lowering that emits such a builtin writes a call no scan sees, the
/// validator is pruned, and every page the lowering touches fails to build for the browser — which
/// `__lgx_owns` did, for every `## Page`. The lowering must reach what it needs without such a
/// builtin (or the loader must key the demand on the lowering's own condition).
#[test]
fn no_emitted_call_is_a_builtin_whose_demand_the_loader_reads_from_the_authors_text() {
    let keyed = builtins_whose_demand_the_loader_reads_from_the_source();
    let emitted: BTreeSet<String> = lgx_lower_source::files()
        .iter()
        .flat_map(|(_, src)| interned_names(src))
        .collect();
    let hidden: Vec<&String> = keyed.iter().filter(|b| emitted.contains(*b)).collect();
    assert!(
        hidden.is_empty(),
        "the LGX lowering emits {hidden:?}, a builtin whose standard-library demand the loader \
         derives from the AUTHOR's text — invisible for a call the compiler writes, so the demand is \
         never made and the browser build refuses the page"
    );
}

#[test]
fn the_loader_keys_a_demand_on_a_builtin_the_transitive_row_reads() {
    // Anti-vacuity for the row above: the scan finds the canonical edge.
    let keyed = builtins_whose_demand_the_loader_reads_from_the_source();
    assert!(
        keyed.contains("text_from_bytes"),
        "the loader's `validUtf8` edge is read: {keyed:?}"
    );
}

#[test]
fn the_extractor_actually_finds_calls() {
    // ★ ANTI-VACUITY. The test above passes trivially if the scan returns nothing — and a scan over
    // someone else's source is exactly the kind that quietly stops matching after a refactor. So
    // pin that it finds a real, substantial set, and two specific members whose emission is not in
    // doubt: `matchRoute`, which every `## Route` lowers to, and `safeUrl`, which every computed URL
    // attribute a view draws goes through.
    let found = emitted_stdlib_calls();
    assert!(
        found.len() >= 15,
        "the extractor found only {} emitted standard-library calls, which means it has stopped \
         matching rather than that the lowerings stopped emitting: {found:?}",
        found.len()
    );
    for expected in ["matchRoute", "safeUrl"] {
        assert!(
            found.contains(expected),
            "the extractor must see `{expected}`: {found:?}"
        );
    }
}

#[test]
fn an_unregistered_call_would_be_caught() {
    // ★ THE OTHER HALF OF THE ANTI-VACUITY GUARD: prove the comparison can FAIL. A name the loader
    // has never heard of must be reported as undeclared — otherwise `loader_declares` could be
    // answering `true` for everything and this file would be decoration.
    assert!(
        !loader_declares("aFunctionTheLoaderHasNeverHeardOf"),
        "`loader_declares` answers true for a name that does not exist, so it cannot detect a \
         missing demand"
    );
    assert!(
        loader_declares("matchRoute"),
        "and it must answer true for one that is registered"
    );
}

#[test]
fn the_standard_library_scan_sees_the_modules() {
    // The intersection is only as good as both halves. If `stdlib_functions` returned nothing,
    // `emitted_stdlib_calls` would be empty and the main test would pass while checking nothing.
    let fns = stdlib_functions();
    assert!(
        fns.len() >= 100,
        "the standard library scan found only {} functions",
        fns.len()
    );
    for expected in ["htmlEscape", "safeUrl", "routeParam"] {
        assert!(
            fns.contains(expected),
            "the standard library defines `{expected}`: not found"
        );
    }
}

/// ★ A CALL THE COMPILER WRITES IS DEMANDED BY EVERY CONSTRUCT THAT WRITES IT. The registration
/// check above asks whether the loader names a call at all; it cannot ask whether the loader names
/// it under the condition that reaches THIS construct. `htmlEscapeText` is registered — for `html
/// of`, a route, a store — and a page's `Head:` is rendered by the same server walk, which escapes
/// every hole through it. A page whose only escaping construct is its head must still run.
#[test]
fn a_head_that_holds_a_hole_runs_on_the_lowered_tier_with_nothing_else_on_the_page_to_demand_its_escaping(
) {
    use logicaffeine_compile::semantics::dom;
    let src = "## Page /\n\nState:\n    Let who be \"Ada\".\n\nHead:\n    <title>Hello {who}</title>\n\nView:\n    <main><h1>hi</h1></main>\n";
    dom::reset();
    let vm = logicaffeine_compile::compile::vm_outcome(src);
    assert_eq!(vm.error, None, "the page runs on the lowered VM:\n{src}");
    let head = dom::document_head().unwrap_or_default();
    assert!(
        head.contains("<title>Hello Ada</title>"),
        "and its head renders the hole: {head}"
    );
}

/// A `-> View` function in a program that draws no page is read as its markup — the server walk's
/// twin, every hole escaped through `htmlEscapeText` — and nothing else in the program names it.
#[test]
fn a_view_shown_with_no_page_to_place_it_runs_with_nothing_else_to_demand_its_escaping() {
    let src = "## To badge (label: Text) -> View:\n    <span class=\"badge\">{label}</span>\n\n## Main\nShow badge(\"a<b\").\n";
    let vm = logicaffeine_compile::compile::vm_outcome(src);
    assert_eq!(vm.error, None, "the program runs on the VM:\n{src}");
    assert_eq!(
        vm.output.trim(),
        "<span class=\"badge\">a&lt;b</span>",
        "its hole escaped"
    );
    let tw = logicaffeine_compile::compile::tw_outcome(src);
    assert_eq!(
        tw.error, None,
        "the program runs on the tree-walker:\n{src}"
    );
    assert_eq!(
        tw.output.trim(),
        vm.output.trim(),
        "and the tree-walker shows the same markup"
    );
}
