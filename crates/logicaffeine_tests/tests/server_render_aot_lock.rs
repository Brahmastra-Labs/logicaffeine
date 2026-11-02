//! ★ LOCK — a server RENDERS COMPONENTS on the tier it is deployed on. ★
//!
//! `largo build` produces an AOT-Rust binary, and that tier refused the whole component model:
//!
//! ```text
//! Stmt::ComponentDef(_) | Stmt::DirectiveDef { .. } | Stmt::Ui(_) | … =>
//!     compile_error!("LGX components, directives and routes have no AOT-Rust lowering yet")
//! ```
//!
//! So a server that declared a single `## Component` did not build. `html of Greeting` worked on
//! the tree-walker and on the VM — `ssr_lock` and `server_surface_lock` both drive those — and
//! produced a binary that could not be compiled. **Per-request server rendering did not exist in
//! production**: a page's markup could only be produced at BUILD time by pre-rendering, so a
//! dashboard, a search result, or anything else whose content depends on the request had no
//! server-rendered form at all.
//!
//! This is `route_aot_lock`'s finding again, one construct along: proven on the tiers the tests
//! use, broken on the tier a server ships as.
//!
//! ★ WHAT MAY AND MAY NOT CROSS. A component lowers into TWO families of function. Its server
//! render, `__serve_<Name>` (and `__html_<Name>` for `html of`), writes tape records into a root no
//! host holds and serializes them — ordinary code on any tier. `__render_`, `__act_`, `__mount_`,
//! `__bind_` and `__patch_` keep instances, listeners and fragments for a live document, which a
//! server does not have. Emitting the second family is what made the refusal look necessary;
//! skipping it is what makes the first family shippable.

use logicaffeine_compile::compile::{compile_to_rust, tw_outcome, vm_outcome};

/// Compile on the DEPLOY tier and read the generated Rust for refusals.
///
/// `compile_to_rust` answers `Ok` for a program it refused — the refusal is a `compile_error!`
/// embedded in the returned string — so an `is_ok()` assertion here would be green on a server
/// that cannot be built. See `route_aot_lock`.
fn builds(src: &str) -> String {
    let rust =
        compile_to_rust(src).unwrap_or_else(|e| panic!("must compile to AOT-Rust: {e:?}\n{src}"));
    let refusals: Vec<&str> = rust
        .lines()
        .filter(|l| l.contains("compile_error!"))
        .collect();
    assert!(
        refusals.is_empty(),
        "the generated Rust carries {} `compile_error!` — it would not build:\n{}\n{src}",
        refusals.len(),
        refusals.join("\n")
    );
    rust
}

/// The markup the reference tier and the VM produce, held equal to each other — so a change that
/// made the BUILD pass by rendering something else fails loudly.
fn answer(src: &str) -> String {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "tree-walker:\n{src}");
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "VM:\n{src}");
    assert_eq!(
        tw.output.trim_end(),
        vm.output.trim_end(),
        "tree-walker vs VM:\n{src}"
    );
    tw.output.trim_end().to_string()
}

fn holds(src: &str, expected: &str) {
    builds(src);
    assert_eq!(answer(src), expected, "{src}");
}

#[test]
fn a_server_that_declares_a_component_builds() {
    // The smallest case, and the one that decided everything: one component, never rendered.
    builds("## Component Card\nView:\n    <p>card</p>\n\n## Main\nShow \"up\".\n");
}

#[test]
fn html_of_a_component_renders_on_the_deploy_tier() {
    holds(
        "## Component Greeting\nState:\n    Let name be \"ada\".\nView:\n    <p>Hi {name}</p>\n\n\
         ## Main\nShow html of Greeting.\n",
        "<p>Hi ada</p>",
    );
}

#[test]
fn a_component_with_props_renders_per_request() {
    // The shape of every real server render: a page built out of pieces, each handed its data.
    // `html of <Name>` names a component to render; its props come from the PLACEMENT, exactly as
    // they do in a browser — so a parent is what supplies them on either tier.
    holds(
        "## Component Row (label: Text)\nView:\n    <li>{label}</li>\n\n\
         ## Component List\nState:\n    Let items be [\"milk\", \"eggs\"].\n\
         View:\n    <ul>{for each i in items:}<Row label={i}/>{end}</ul>\n\n\
         ## Main\nShow html of List.\n",
        "<ul><li>milk</li><li>eggs</li></ul>",
    );
}

#[test]
fn a_view_route_may_place_a_component() {
    // `## Route … -> View:` is the sentence that makes per-request SSR a thing you can write. A
    // route that cannot place a component can only serve markup nobody factored.
    holds(
        "## Component Card (title: Text)\nView:\n    <li>{title}</li>\n\n\
         ## Route GET /report -> View:\n    <ul><Card title={\"q3\"}/></ul>\n\n\
         ## Main\nShow route(\"GET\", \"/report\").\n",
        "<ul><li>q3</li></ul>",
    );
}

#[test]
fn a_component_with_state_and_control_flow_renders() {
    holds(
        "## Component List\nState:\n    Let items be [\"a\", \"b\"].\n\
         View:\n    <ul>{for each i in items:}<li>{i}</li>{else:}<li>none</li>{end}</ul>\n\n\
         ## Main\nShow html of List.\n",
        "<ul><li>a</li><li>b</li></ul>",
    );
}

#[test]
fn a_component_a_server_cannot_render_is_still_refused_by_name() {
    // ★ THE REFUSAL MUST NOT BECOME SILENCE. Deleting the blanket `compile_error!` is right; a
    // server that quietly drops a page it cannot draw is not. `Mount` needs a document, so it has
    // no server form — and saying so beats building a binary whose page never appears.
    let refused = compile_to_rust("## Component App\nView:\n    <p>x</p>\n\n## Main\nMount App.\n");
    let err = format!(
        "{:?}",
        refused.expect_err("mounting must be refused on a tier with no document")
    );
    assert!(
        err.to_lowercase().contains("dom") || err.to_lowercase().contains("document"),
        "…and the refusal NAMES the missing document, so the reader knows the tier is wrong for \
         the program rather than the program being wrong: {err}"
    );
}

#[test]
fn the_browser_only_lowering_never_reaches_the_server_binary() {
    // `__render_` and friends call `createElement`. If they were emitted, the generated Rust would
    // name a function that tier has no definition for — which is the failure this lock exists to
    // keep out, in the form it would come back in.
    let rust = builds(
        "## Component Greeting\nState:\n    Let name be \"ada\".\nView:\n    <p>Hi {name}</p>\n\n\
         ## Main\nShow html of Greeting.\n",
    );
    assert!(
        !rust.contains("createElement"),
        "a server binary must carry no document calls:\n{rust}"
    );
    assert!(
        rust.contains("__html_Greeting") || rust.contains("__html_greeting"),
        "…and must carry the one function that renders markup:\n{rust}"
    );
}
