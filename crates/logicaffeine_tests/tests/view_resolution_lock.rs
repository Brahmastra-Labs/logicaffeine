//! LOCK — every name a view uses resolves where it is written, or the program is refused with
//! the caret on that name and the name it most likely meant.
//!
//! One resolution pass (`parser::lgx_resolve`) walks every view — a component's `View:` and
//! `Head:`, a route template, a `-> View` function — with the scope its tiers give it. A value
//! name in a hole of any shape, a condition, an iterable, a key, an attribute, a class condition,
//! a `{let}`, a prop value or a handler argument; an `on-<event>` target; a `bind`; a placed
//! component; an `{extends}`: each is refused when nothing binds it.

#[path = "language_common/mod.rs"]
mod common;

use common::{accepted, refusal};

/// A component `App` with `state`, `actions` and `view`, mounted.
fn app(state: &str, actions: &str, view: &str) -> String {
    let state = if state.is_empty() {
        String::new()
    } else {
        format!("State:\n{state}")
    };
    let actions = if actions.is_empty() {
        String::new()
    } else {
        format!("Actions:\n{actions}")
    };
    format!("## Component App\n{state}{actions}View:\n    {view}\n\n## Main\nMount App.\n")
}

const NAME: &str = "    Let name be \"ada\".\n";
const TODOS: &str = "    Let todos be [\"a\", \"b\"].\n";
const ADD: &str =
    "    To add:\n        Set n to n + 1.\n    To remove (t: Text):\n        Set n to n + 1.\n";
const N: &str = "    Let n be 0.\n";

/// The refusal `src` produces must cover exactly `covered` and carry every fragment.
fn refused_at(src: &str, covered: &str, fragments: &[&str]) {
    let (message, at) = refusal(src);
    assert_eq!(at, covered, "the caret must cover `{covered}`: {message}");
    for f in fragments {
        assert!(
            message.contains(f),
            "the message must carry {f:?}: {message}"
        );
    }
}

// ── holes of every shape ────────────────────────────────────────────────────────────────────────

#[test]
fn a_misspelt_hole_is_refused_with_did_you_mean() {
    refused_at(
        &app(NAME, "", "<p>{nme}</p>"),
        "nme",
        &[
            "`nme` is not defined in `App`'s view",
            "did you mean `name`?",
        ],
    );
}

#[test]
fn a_hole_among_text_is_refused_the_same_way() {
    refused_at(
        &app(NAME, "", "<p>hello {nme}!</p>"),
        "nme",
        &["did you mean `name`?"],
    );
}

#[test]
fn a_possessive_hole_resolves_its_subject() {
    refused_at(
        &app(NAME, "", "<p>{nme's length}</p>"),
        "nme",
        &["did you mean `name`?"],
    );
}

#[test]
fn a_filtered_hole_resolves_its_value_and_its_arguments() {
    refused_at(
        &app(NAME, "", "<p>{nme | upper}</p>"),
        "nme",
        &["did you mean `name`?"],
    );
    refused_at(
        &app(
            "    Let name be \"ada\".\n    Let width be 8.\n",
            "",
            "<p>{name | pad widht}</p>",
        ),
        "widht",
        &["did you mean `width`?"],
    );
}

#[test]
fn a_hole_far_from_every_name_is_refused_without_a_guess() {
    let (message, at) = refusal(&app(NAME, "", "<p>{zzzzzzzz}</p>"));
    assert_eq!(at, "zzzzzzzz");
    assert!(!message.contains("did you mean"), "{message}");
    assert!(
        message.contains("`zzzzzzzz` is not defined in `App`'s view."),
        "{message}"
    );
}

#[test]
fn a_condition_an_iterable_and_a_key_are_resolved() {
    refused_at(
        &app("    Let flag be true.\n", "", "{if flg:}<p>x</p>{end}"),
        "flg",
        &["did you mean `flag`?"],
    );
    refused_at(
        &app(
            "    Let flag be true.\n",
            "",
            "{if flag:}<p>x</p>{else if flg:}<p>y</p>{end}",
        ),
        "flg",
        &["did you mean `flag`?"],
    );
    refused_at(
        &app(TODOS, "", "<ul>{for each t in todo:}<li>{t}</li>{end}</ul>"),
        "todo",
        &["did you mean `todos`?"],
    );
    refused_at(
        &app(
            TODOS,
            "",
            "<ul>{for each t in todos, keyed by u:}<li>{t}</li>{end}</ul>",
        ),
        "u",
        &["`u` is not defined"],
    );
}

#[test]
fn attributes_and_class_conditions_are_resolved() {
    refused_at(
        &app(NAME, "", "<p title={nme}>x</p>"),
        "nme",
        &["did you mean `name`?"],
    );
    refused_at(
        &app(NAME, "", "<p title=\"hi {nme}\">x</p>"),
        "nme",
        &["did you mean `name`?"],
    );
    refused_at(
        &app("    Let done be true.\n", "", "<p class:done={dne}>x</p>"),
        "dne",
        &["did you mean `done`?"],
    );
}

#[test]
fn a_let_value_is_resolved() {
    refused_at(
        &app(NAME, "", "{let shout be nme}<p>{shout}</p>"),
        "nme",
        &["did you mean `name`?"],
    );
}

// ── template scope ──────────────────────────────────────────────────────────────────────────────

#[test]
fn a_loop_item_is_in_scope_only_inside_its_loop() {
    accepted(&app(
        TODOS,
        "",
        "<ul>{for each t in todos, at index i:}<li>{i}{t}</li>{end}</ul>",
    ));
    refused_at(
        &app(
            TODOS,
            "",
            "<ul>{for each t in todos:}<li>{t}</li>{end}</ul><p>{t}</p>",
        ),
        "t",
        &["`t` is not defined"],
    );
    refused_at(
        &app(
            TODOS,
            "",
            "<ul>{for each t in todos:}<li>{t}</li>{else:}<li>{t}</li>{end}</ul>",
        ),
        "t",
        &["`t` is not defined"],
    );
}

#[test]
fn a_let_is_in_scope_after_it_and_not_before() {
    accepted(&app(NAME, "", "{let shout be name}<p>{shout}</p>"));
    accepted(&app(
        NAME,
        "",
        "<div>{let shout be name}</div><p>{shout}</p>",
    ));
    refused_at(
        &app(NAME, "", "<p>{shout}</p>{let shout be name}"),
        "shout",
        &["`shout` is not defined"],
    );
    refused_at(
        &app(
            "    Let on be true.\n",
            "",
            "{if on:}{let shout be 1}{end}<p>{shout}</p>",
        ),
        "shout",
        &["`shout` is not defined"],
    );
}

#[test]
fn an_inspect_arm_binds_its_names_for_its_body() {
    accepted(&app(
        "    Let v be some 3.\n",
        "",
        "{inspect v:}{when Some (k):}<p>{k}</p>{when None:}<p>-</p>{end}",
    ));
    refused_at(
        &app(
            "    Let v be some 3.\n",
            "",
            "{inspect v:}{when Some (k):}<p>{k}</p>{when None:}<p>{k}</p>{end}",
        ),
        "k",
        &["`k` is not defined"],
    );
}

// ── the component's own names, and the program's ────────────────────────────────────────────────

#[test]
fn state_props_derived_and_page_parameters_resolve() {
    accepted(&app(
        "    Let n be 2.\n    Let twice always be n * 2.\n",
        "",
        "<p>{n} {twice}</p>",
    ));
    accepted("## Component Row (label: Text)\nView:\n    <li>{label}</li>\n\n## Component App\nView:\n    <ul><Row label=\"a\"/></ul>\n\n## Main\nMount App.\n");
    accepted("## Page /posts/:id (id: Int)\nView:\n    <p>{id}</p>\n");
}

#[test]
fn a_page_is_named_by_its_header() {
    refused_at(
        "## Page /about\nState:\n    Let name be \"a\".\nView:\n    <p>{nme}</p>\n",
        "nme",
        &["`## Page /about`'s view"],
    );
    let (message, _) = refusal("## Page /about\nView:\n    <p>{nme}</p>\n");
    assert!(
        !message.contains("__page"),
        "an internal name never reaches the author: {message}"
    );
}

#[test]
fn program_definitions_resolve() {
    accepted("## To shout (s: Text) -> Text:\n    Return s.\n\n## Component App\nView:\n    <p>{shout(\"a\")} {greeting}</p>\n\n## Main\nLet greeting be \"hi\".\nMount App.\n");
    accepted(&app("", "", "<p>{today} {now}</p>"));
    accepted("## A Color is one of:\n    a Red.\n    a Green.\n\n## Component App\nState:\n    Let c be Red.\nView:\n    <p>{c is Red}</p>\n\n## Main\nMount App.\n");
}

#[test]
fn a_call_to_a_name_the_parser_cannot_see_is_left_to_the_compile_seams() {
    // Builtins and demand-spliced library functions are not visible to the parser; the call NAME is
    // resolved once the prelude is in. Its arguments are resolved here.
    accepted(&app(NAME, "", "<p>{format(name)} {name | upper}</p>"));
    refused_at(
        &app(NAME, "", "<p>{format(nme)}</p>"),
        "nme",
        &["did you mean `name`?"],
    );
}

#[test]
fn a_store_is_read_after_use() {
    let store = "## Store Session\nState:\n    Let user be \"ada\".\n\n";
    accepted(&format!("{store}## Component App\nUse Session.\nView:\n    <p>{{Session's user}}</p>\n\n## Main\nMount App.\n"));
    refused_at(
        &format!("{store}## Component App\nView:\n    <p>{{Session's user}}</p>\n\n## Main\nMount App.\n"),
        "Session",
        &["`Session` is a store: read it after `Use Session.`"],
    );
}

#[test]
fn a_child_does_not_see_its_callers_names() {
    refused_at(
        "## Component Child\nView:\n    <p>{secret}</p>\n\n## Component App\nState:\n    Let secret be \"s\".\nView:\n    <div><Child/></div>\n\n## Main\nMount App.\n",
        "secret",
        &["`secret` is not defined in `Child`'s view"],
    );
}

// ── handlers and binds ──────────────────────────────────────────────────────────────────────────

#[test]
fn a_misspelt_handler_is_refused_at_the_handler() {
    refused_at(
        &app(N, ADD, "<button on-click={ad}>+</button>"),
        "ad",
        &[
            "`ad` is not an action of `App` — did you mean `add`?",
            "its actions are `add`, `remove`",
        ],
    );
}

#[test]
fn a_handler_arguments_are_resolved() {
    refused_at(
        &app(
            "    Let n be 0.\n    Let todo be \"t\".\n",
            ADD,
            "<button on-click={remove(tdo)}>x</button>",
        ),
        "tdo",
        &["did you mean `todo`?"],
    );
}

#[test]
fn a_handler_on_a_component_with_no_actions_says_so() {
    refused_at(
        &app(N, "", "<button on-click={go}>+</button>"),
        "go",
        &["`App` declares no actions"],
    );
}

#[test]
fn a_callback_prop_answers_an_event() {
    accepted("## Component Row (label: Text) and (pick: fn(Text) -> Unit)\nView:\n    <li on-click={pick(label)}>{label}</li>\n\n## Component App\nState:\n    Let n be 0.\nActions:\n    To take (t: Text):\n        Set n to n + 1.\nView:\n    <ul><Row label=\"a\" pick={take}/></ul>\n\n## Main\nMount App.\n");
    refused_at(
        "## Component Row (label: Text)\nView:\n    <li on-click={label}>{label}</li>\n\n## Component App\nView:\n    <ul><Row label=\"a\"/></ul>\n\n## Main\nMount App.\n",
        "label",
        &["`label` is a prop of `Row` holding a `Text`"],
    );
}

#[test]
fn a_bind_names_the_components_own_state() {
    accepted(&app(
        "    Let draft be \"\".\n",
        "",
        "<input aria-label=\"d\" bind={draft}>",
    ));
    refused_at(
        &app(
            "    Let draft be \"\".\n",
            "",
            "<input aria-label=\"d\" bind={drft}>",
        ),
        "drft",
        &["did you mean `draft`?"],
    );
    refused_at(
        "## Component Field (value: Text)\nView:\n    <input aria-label=\"v\" bind={value}>\n\n## Component App\nView:\n    <Field value=\"a\"/>\n\n## Main\nMount App.\n",
        "value",
        &["`value` is a prop of `Field`"],
    );
}

// ── placements and layouts ──────────────────────────────────────────────────────────────────────

const ROW: &str =
    "## Component Row (label: Text) and (done: Bool)\nView:\n    <li>{label}</li>\n\n";

#[test]
fn a_placed_component_that_does_not_exist_is_named() {
    let nav = "## Component NavBar\nView:\n    <nav>n</nav>\n\n";
    refused_at(
        &format!("{nav}{}", app("", "", "<Nav/>")),
        "Nav",
        &["`<Nav/>` names no component.", "declares `App`, `NavBar`"],
    );
    refused_at(
        &format!("{nav}{}", app("", "", "<NavBr/>")),
        "NavBr",
        &["`<NavBr/>` names no component — did you mean `NavBar`?"],
    );
}

#[test]
fn an_extended_layout_that_does_not_exist_is_named() {
    let layout = "## Component Layout\nView:\n    <main>{block body}</main>\n\n";
    let (message, _) = refusal(&format!("{layout}## Component App\nView:\n    {{extends Layot}}\n    {{block body:}}<p>x</p>{{end}}\n\n## Main\nMount App.\n"));
    assert!(
        message.contains("`{extends Layot}` names no component — did you mean `Layout`?"),
        "{message}"
    );
}

#[test]
fn a_missing_prop_points_at_the_placement() {
    refused_at(
        &format!("{ROW}{}", app("", "", "<ul><Row label=\"a\"/></ul>")),
        "Row",
        &["`<Row>` does not pass `done`"],
    );
}

#[test]
fn an_unknown_prop_points_at_the_attribute() {
    refused_at(
        &format!(
            "{ROW}{}",
            app(
                "",
                "",
                "<ul><Row label=\"a\" done={true} dne={false}/></ul>"
            )
        ),
        "dne={false}",
        &["has no prop `dne`"],
    );
}

#[test]
fn a_mistyped_prop_points_at_the_attribute() {
    refused_at(
        &format!(
            "{ROW}{}",
            app("", "", "<ul><Row label=\"a\" done=\"yes\"/></ul>")
        ),
        "done=\"yes\"",
        &["is a `Bool`"],
    );
}

// ── other views ─────────────────────────────────────────────────────────────────────────────────

#[test]
fn a_head_is_resolved() {
    accepted("## Page /\nState:\n    Let n be 1.\nHead:\n    <title>{n} left</title>\nView:\n    <p>{n}</p>\n");
    refused_at("## Page /\nState:\n    Let n be 1.\nHead:\n    <title>{nn} left</title>\nView:\n    <p>{n}</p>\n", "nn", &["did you mean `n`?"]);
}

#[test]
fn a_route_template_is_resolved_with_its_parameters() {
    accepted("## Route GET /p/:id (id: Int) -> View:\n    <p>{id}</p>\n");
    refused_at(
        "## Route GET /p/:id (id: Int) -> View:\n    <p>{idd}</p>\n",
        "idd",
        &["`## Route GET /p/:id`'s view", "did you mean `id`?"],
    );
}

#[test]
fn a_view_function_is_resolved_with_its_parameters() {
    accepted("## To badge (label: Text) -> View:\n    <b>{label}</b>\n\n## Main\nShow \"x\".\n");
    refused_at(
        "## To badge (label: Text) -> View:\n    <b>{lable}</b>\n\n## Main\nShow \"x\".\n",
        "lable",
        &["`## To badge`'s view", "did you mean `label`?"],
    );
}
