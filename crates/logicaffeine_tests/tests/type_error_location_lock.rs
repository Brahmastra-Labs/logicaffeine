//! A type error says where it is, in words its author wrote — never a name the compiler minted.
//!
//! The checker runs after lowering, where a route or a component action has become a function whose
//! name (`__route_GET__agent`, `__act_Counter_add`, `__lgx_route_for`) appears nowhere in the
//! source, and a type error used to reach the author with no location at all. It now names the
//! function the author wrote (`in `f``), or the construct a compiler-written function was lowered
//! from (`in `## Route GET /agent``, `in the `## Route` table`). Whatever path produced it, no
//! rendered message may contain a `__` name.

use logicaffeine_base::Interner;
use logicaffeine_compile::compile::compile_program_full;
use logicaffeine_language::error::socratic_explanation;

fn type_error(src: &str) -> Option<String> {
    compile_program_full(src)
        .err()
        .map(|e| socratic_explanation(&e, &Interner::new()))
}

/// Programs that must be refused, each with the place its message must name (`None`: a `## Main`
/// statement, which names no place).
const REFUSED: &[(&str, &str, Option<&str>)] = &[
    (
        "a function the author wrote",
        "## To f (n: Int) -> Text:\n    Let t: Text be n + 1.\n    Return t.\n\n## Main\nShow f(1).\n",
        Some("in `f`"),
    ),
    (
        "a route handler's own body",
        "## Route GET /agent (name: Text) -> Text:\n    Let n be 5.\n    Let t: Text be n + 1.\n    Return \"hello {name}\".\n\n## Main\nShow route(\"GET\", \"/agent?name=ada\").\n",
        Some("in `## Route GET /agent`"),
    ),
    ("a `## Main` statement", "## Main\nLet t: Text be 1 + 1.\nShow t.\n", None),
];

/// Programs checked only for the property: if one is refused, its message names no compiler-written
/// function. (A `Request` route parameter reads the route table's own request — a shape the lowering
/// has mistyped before.)
const PROPERTY_ONLY: &[(&str, &str)] = &[(
    "a route reading the request's headers",
    "## Route GET /agent (request: Request) -> Text:\n    If hasHeader(request, \"user-agent\"):\n        Return \"agent: \" + headerOf(request, \"user-agent\").\n    Return \"no agent\".\n\n## Main\nShow route(\"GET\", \"/agent\").\n",
)];

#[test]
fn a_type_error_names_the_place_its_author_wrote() {
    for (what, src, place) in REFUSED {
        let msg =
            type_error(src).unwrap_or_else(|| panic!("{what}: the program must be refused\n{src}"));
        match place {
            Some(place) => assert!(
                msg.contains(place),
                "{what}: the message must say `{place}`:\n{msg}"
            ),
            None => assert!(
                !msg.contains("(in "),
                "{what}: a `## Main` statement names no place:\n{msg}"
            ),
        }
    }
}

#[test]
fn no_rendered_type_error_names_a_compiler_written_function() {
    let all = REFUSED
        .iter()
        .map(|(what, src, _)| (*what, *src))
        .chain(PROPERTY_ONLY.iter().copied());
    for (what, src) in all {
        if let Some(msg) = type_error(src) {
            assert!(
                !msg.contains("__"),
                "{what}: the message names a compiler-written function:\n{msg}"
            );
        }
    }
}
