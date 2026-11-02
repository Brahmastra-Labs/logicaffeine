//! ★ LOCK — a page's CHILD COMPONENTS render on the server. ★
//!
//! A page is built out of components: a row, a card, a field. The server-side renderer covered
//! elements, text, holes, `{if}`, `{for each}` and `{let}` — and REFUSED a child component, which
//! means it refused every real page.
//!
//! ★ WHAT THAT COST, and why it is not a small gap: `largo build` PRE-RENDERS every page into its
//! document (T6.8), and pre-rendering is an improvement that never fails — a page the renderer
//! cannot draw ships its shell and works in the browser. So the failure was SILENT and total: the
//! flagship example, whose list is a `<TodoRow/>` per item, shipped a document with one `<script>`
//! tag in it. A crawler saw nothing, a slow connection saw nothing, and scripts-off saw nothing —
//! for a framework whose whole premise is that the page works before its module loads.
//!
//! ★ IT IS A CALL, NOT AN INLINING — `__html_<Child>(props…)`, exactly as the browser tier calls
//! `__render_<Child>(parent, props…)`. Same tree, different destination, same argument order (the
//! child's DECLARED prop order), so a call site may name its props in any order it likes.
//!
//! ★ AND THE DEMAND FOLLOWS THE TREE. `html of App` is the only thing an author writes; the rows
//! it places are named nowhere. A rule that fired only on the spelled name emitted the page's own
//! renderer and none of the children it calls — a call to a function that does not exist. The seed
//! is what the source says; the set is its CLOSURE, the same shape the JSON derive uses.

#![cfg(feature = "wasm-jit")]

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

const TODO: &str = "## A Todo has:\n    an id, which is Int.\n    a text, which is Text.\n    a done, which is Bool.\n\n";

/// The markup both tiers render, held equal.
fn html(src: &str) -> String {
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

#[test]
fn a_child_component_renders_into_its_parents_markup() {
    // ★ THE HEADLINE — the child's own elements appear inside the parent's, in place.
    let src = "## Component Badge\n\
       \n\
       View:\n\
      \x20   <span class=\"badge\">new</span>\n\
       \n\
       ## Component App\n\
       \n\
       View:\n\
      \x20   <div><Badge/></div>\n\
       \n\
       ## Main\n\
       Show html of App.\n";
    assert_eq!(html(src), "<div><span class=\"badge\">new</span></div>");
}

#[test]
fn a_child_reads_the_prop_it_was_handed() {
    // A child that ignored its props would render the same markup for every row.
    let src = format!(
        "{TODO}## Component Row (todo: Todo)\n\
         \n\
         View:\n\
        \x20   <li>{{todo's text}}</li>\n\
         \n\
         ## Component App\n\
         \n\
         State:\n\
        \x20   Let todos be [a new Todo with id 1 and text \"milk\" and done false, a new Todo with id 2 and text \"eggs\" and done true].\n\
         \n\
         View:\n\
        \x20   <ul>{{for each t in todos:}}<Row todo={{t}}/>{{end}}</ul>\n\
         \n\
         ## Main\n\
         Show html of App.\n"
    );
    assert_eq!(html(&src), "<ul><li>milk</li><li>eggs</li></ul>");
}

#[test]
fn props_are_matched_by_name_not_by_position() {
    // ★ The call site writes props in whatever order reads well; the ARGUMENTS are built in the
    // child's declared order. Getting this wrong swaps two values of the same type silently.
    let src = "## Component Pair (left: Text) and (right: Text)\n\
       \n\
       View:\n\
      \x20   <p>{left}|{right}</p>\n\
       \n\
       ## Component App\n\
       \n\
       View:\n\
      \x20   <div><Pair right=\"R\" left=\"L\"/></div>\n\
       \n\
       ## Main\n\
       Show html of App.\n";
    assert_eq!(html(src), "<div><p>L|R</p></div>");
}

#[test]
fn a_hole_in_a_child_is_escaped_exactly_once() {
    // ★ THE SECURITY EDGE. The child returns MARKUP it escaped itself, hole by hole; the parent
    // appends that answer RAW. Escaping it a second time would put `&amp;lt;` on the page (visible
    // nonsense); escaping it zero times would run a stranger's script. Both are asserted by
    // pinning the exact bytes.
    let src = "## Component Note (body: Text)\n\
       \n\
       View:\n\
      \x20   <p>{body}</p>\n\
       \n\
       ## Component App\n\
       \n\
       View:\n\
      \x20   <div><Note body=\"<img src=x onerror=alert(1)>\"/></div>\n\
       \n\
       ## Main\n\
       Show html of App.\n";
    assert_eq!(
        html(src),
        "<div><p>&lt;img src=x onerror=alert(1)&gt;</p></div>",
        "escaped once: the child escapes its hole, the parent appends the child's markup as markup"
    );
}

#[test]
fn a_grandchild_renders_too() {
    // The demand rule is a CLOSURE, not one hop: a child places a child of its own, and nothing in
    // the source names either.
    let src = "## Component Leaf\n\
       \n\
       View:\n\
      \x20   <b>leaf</b>\n\
       \n\
       ## Component Mid\n\
       \n\
       View:\n\
      \x20   <span><Leaf/></span>\n\
       \n\
       ## Component App\n\
       \n\
       View:\n\
      \x20   <div><Mid/></div>\n\
       \n\
       ## Main\n\
       Show html of App.\n";
    assert_eq!(html(src), "<div><span><b>leaf</b></span></div>");
}

#[test]
fn a_child_with_its_own_state_binds_it() {
    // A child is a component, not a fragment: its `State:` is bound before its view renders, in
    // the same order the runtime binds it (plain fields, then derived).
    let src = "## Component Counter\n\
       \n\
       State:\n\
      \x20   Let n be 3.\n\
      \x20   Let twice always be n * 2.\n\
       \n\
       View:\n\
      \x20   <span>{n}/{twice}</span>\n\
       \n\
       ## Component App\n\
       \n\
       View:\n\
      \x20   <div><Counter/></div>\n\
       \n\
       ## Main\n\
       Show html of App.\n";
    assert_eq!(html(src), "<div><span>3/6</span></div>");
}

#[test]
fn a_child_the_server_cannot_draw_refuses_the_whole_page() {
    // ★ A PAGE RENDERED SHORT IS WORSE THAN ONE REFUSED — the rule the server walk is built on,
    // held at the CHILD boundary.
    //
    // ★ THE SUBJECT MOVED, AND FOR THE RIGHT REASON. This used to place a child carrying a `bind`,
    // on the premise that "a `bind` is browser behaviour with no served form". That premise is no
    // longer true: a `bind` now RENDERS the field's current value — `value="…"`, or the bare
    // `checked` whose state is its presence — which is what a server-rendered form IS. A reader who
    // never loaded the app sees what is in the fields, and a browser that hydrates finds the values
    // it would have written. `ssr_subset_lock::a_bound_field_renders_its_current_value` holds that,
    // so this file stopped measuring a refusal and started measuring nothing.
    //
    // `{raw}` is the subject now, and it is outside the subset by DECISION rather than by absence:
    // interpolating unescaped markup is a judgement about trust, not a rendering. The boundary
    // rule is unchanged — what the child cannot draw, the whole page refuses.
    let src = "## Component Field\n\
       \n\
       State:\n\
      \x20   Let h be \"<b>hi</b>\".\n\
       \n\
       View:\n\
      \x20   <div>{raw h}</div>\n\
       \n\
       ## Component App\n\
       \n\
       View:\n\
      \x20   <div><Field/></div>\n\
       \n\
       ## Main\n\
       Show html of App.\n";
    let out = tw_outcome(src);
    assert!(
        out.error.is_some(),
        "a page whose child cannot be served must be refused, not rendered short"
    );

    // ★ AND THE OTHER HALF, WHICH THE OLD FIXTURE COULD NOT STATE: a child the server CAN draw is
    // drawn, all the way through. Without this, re-pointing the subject above could have hidden a
    // walk that had quietly started refusing everything.
    let served = "## Component Field\n\
       \n\
       State:\n\
      \x20   Let draft be \"notes\".\n\
       \n\
       View:\n\
      \x20   <input bind={draft}>\n\
       \n\
       ## Component App\n\
       \n\
       View:\n\
      \x20   <div><Field/></div>\n\
       \n\
       ## Main\n\
       Show html of App.\n";
    let out = tw_outcome(served);
    assert_eq!(out.error, None, "a child the server CAN draw must be drawn");
    assert!(
        out.output.contains("value=\"notes\""),
        "and drawn in full — the bound value travels through the child boundary: {}",
        out.output
    );
}
