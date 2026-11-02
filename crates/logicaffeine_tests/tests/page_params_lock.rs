//! ★ LOCK — a page's `:name` path segments BIND. ★
//!
//! `## Page /posts/:id (id: Text)` is a header the parser accepts and documents:
//! *"each `:name` segment of the path binds a typed parameter the page's state can
//! read"* (`parser/component.rs:113-114`). The parameter list was parsed, carried
//! through the AST as `ComponentKind::Page { path, params }` — and **nothing ever read
//! `params`**. Every consumer destructures `path` and spells the rest `..`.
//!
//! ★ THIS IS THE SAME BUG THE PATH ITSELF HAD, IN THE SAME ENUM VARIANT. `web_bundle.rs`
//! carries a comment recording that `Page { path }` was "carried and never read" and
//! fixing it; `params` sat one field away, unexercised, the whole time. That is this
//! repo's most-repeated shape — a fix lands where the bug was OBSERVED while the
//! identical flaw sits in a parallel spelling — and it is why this lock pins the
//! BINDING rather than any one page.
//!
//! ★ NOTHING NEW IS NEEDED TO MATCH. `web.lg` already ships `matchRoute(pattern, path)`
//! and `routeParam(pattern, name, path)`, documented with the exact example
//! `routeParam("/todos/:id", "id", "/todos/42")` is `"42"`. The whole matching half was
//! built and simply never wired to the header that declares the pattern.
//!
//! A detail page — `/posts/:id`, `/users/:id` — is table stakes for a site. Without this
//! the author writes the sentence, is told it was understood, and reads an empty string.

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::semantics::dom;

/// Boot a page AT `path` (a deep link / refresh) and return its rendered body.
fn rendered_at(path: &str, src: &str) -> String {
    dom::reset();
    dom::init_route(path, "");
    let out = tw_outcome(src);
    assert_eq!(
        out.error, None,
        "the page must run cleanly at {path}:\n{src}"
    );
    let html = dom::render();
    html.strip_prefix("<body>")
        .and_then(|h| h.strip_suffix("</body>"))
        .unwrap_or(&html)
        .to_string()
}

#[test]
fn a_declared_path_parameter_is_bound_from_the_url() {
    let src = "## Page /posts/:id (id: Text)\nView:\n    <p id=\"out\">post {id}</p>\n";
    let html = rendered_at("/posts/42", src);
    assert!(
        html.contains("post 42"),
        "`:id` must bind the segment the visitor asked for, not an empty string:\n{html}"
    );
}

#[test]
fn two_parameters_bind_to_their_own_segments() {
    // Position matters: a single-parameter fix that reads "the first `:name`" would pass
    // the test above and put the year in the slug.
    let src = "## Page /blog/:year/:slug (year: Text) and (slug: Text)\n\
               View:\n    <p id=\"out\">{year}|{slug}</p>\n";
    let html = rendered_at("/blog/2026/hello", src);
    assert!(
        html.contains("2026|hello"),
        "each name takes ITS OWN segment:\n{html}"
    );
}

#[test]
fn a_typed_parameter_arrives_as_that_type() {
    // `(id: Int)` is a declaration, not decoration — the page does arithmetic on it.
    let src = "## Page /posts/:id (id: Int)\nView:\n    <p id=\"out\">next {id + 1}</p>\n";
    let html = rendered_at("/posts/7", src);
    assert!(
        html.contains("next 8"),
        "an `Int` parameter must be a number, not text:\n{html}"
    );
}

#[test]
fn a_parameter_the_path_does_not_declare_is_refused() {
    // The reverse mistake: a parameter list naming something the path has no `:slot` for
    // can never be filled, so it is a mistake to accept it silently.
    let src = "## Page /posts/:id (id: Text) and (missing: Text)\n\
               View:\n    <p>{id}{missing}</p>\n";
    dom::reset();
    dom::init_route("/posts/1", "");
    let out = tw_outcome(src);
    let said = out.error.unwrap_or_default();
    assert!(
        said.contains("missing"),
        "a parameter with no `:missing` segment in the path must be refused BY NAME, \
         never bound to nothing; said: {said:?}"
    );
}
