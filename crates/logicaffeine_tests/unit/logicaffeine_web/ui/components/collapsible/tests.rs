use super::*;
use crate::ui::components::chevron::CHEVRON_SVG;

#[derive(Props, Clone, PartialEq)]
struct HostProps {
    title: &'static str,
    open: bool,
    id: Option<&'static str>,
}

/// One block, owned the way a page owns it: the page keeps the signal.
#[allow(non_snake_case)]
fn Host(props: HostProps) -> Element {
    let open = use_signal(|| props.open);
    rsx! {
        Collapsible {
            title: props.title,
            open,
            id: props.id.map(str::to_string),
            p { class: "block-content", "What the block holds." }
        }
    }
}

fn render(title: &'static str, open: bool, id: Option<&'static str>) -> String {
    let mut vdom = VirtualDom::new_with_props(Host, HostProps { title, open, id });
    vdom.rebuild_in_place();
    dioxus_ssr::render(&vdom)
}

/// The opening tag of the first element whose opening tag contains `needle`.
fn tag_with<'a>(html: &'a str, needle: &str) -> Option<&'a str> {
    let at = html.find(needle)?;
    let start = html[..at].rfind('<')?;
    let end = at + html[at..].find('>')?;
    Some(&html[start..=end])
}

/// The first opening tag of `name`.
fn tag<'a>(html: &'a str, name: &str) -> Option<&'a str> {
    tag_with(html, &format!("<{name}"))
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let at = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    tag[at..].split('"').next()
}

fn has_bool_attr(tag: &str, name: &str) -> bool {
    tag.contains(&format!(" {name}=true")) || tag.contains(&format!(" {name} ")) || tag.contains(&format!(" {name}>"))
}

/// The body the heading names, by its id.
fn body<'a>(html: &'a str) -> &'a str {
    let heading = tag(html, "button").expect("the block has a button heading");
    let id = attr(heading, "aria-controls").expect("the heading names the body it controls");
    tag_with(html, &format!("id=\"{id}\"")).unwrap_or_else(|| panic!("no element has the id {id:?} the heading controls:\n{html}"))
}

#[test]
fn a_closed_block_is_a_button_heading_that_says_it_is_closed() {
    let html = render("Syntax Tree", false, None);
    let heading = tag(&html, "button").unwrap_or_else(|| panic!("a closed block has no button heading:\n{html}"));
    assert_eq!(attr(heading, "type"), Some("button"), "the heading is not a plain button: {heading}");
    assert_eq!(attr(heading, "aria-expanded"), Some("false"), "a closed block's heading says {heading}");
}

#[test]
fn an_open_block_says_so_and_shows_its_body() {
    let html = render("Symbol Dictionary", true, None);
    let heading = tag(&html, "button").unwrap_or_else(|| panic!("an open block has no button heading:\n{html}"));
    assert_eq!(attr(heading, "aria-expanded"), Some("true"), "an open block's heading says {heading}");
    assert!(!has_bool_attr(body(&html), "hidden"), "an open block hides its body: {}", body(&html));
    assert!(html.contains("What the block holds."), "an open block does not show its content:\n{html}");
}

#[test]
fn the_heading_controls_the_body_it_names() {
    let html = render("Syntax Tree", true, None);
    let heading = tag(&html, "button").expect("a button heading");
    let body = body(&html);
    assert_eq!(attr(body, "role"), Some("region"), "the body is not a region: {body}");
    let heading_id = attr(heading, "id").unwrap_or_else(|| panic!("the heading has no id to label its body: {heading}"));
    assert_eq!(attr(body, "aria-labelledby"), Some(heading_id), "the body is not labelled by its heading: {body}");
    let inside = &html[html.find(body).unwrap()..];
    assert!(inside.contains("What the block holds."), "the content is not inside the body the heading controls:\n{html}");
}

#[test]
fn a_closed_body_stays_in_the_page_hidden() {
    let html = render("Syntax Tree", false, None);
    let body = body(&html);
    assert!(has_bool_attr(body, "hidden"), "a closed block's body is not hidden: {body}");
    let inside = &html[html.find(body).unwrap()..];
    assert!(inside.contains("What the block holds."), "a closed block drops its content from the page:\n{html}");
}

#[test]
fn the_chevron_comes_before_the_title() {
    let html = render("Symbol Dictionary", false, None);
    let heading_at = html.find("<button").expect("a button heading");
    let heading = &html[heading_at..heading_at + html[heading_at..].find("</button>").expect("the heading closes")];
    let chevron = heading.find("<svg").unwrap_or_else(|| panic!("the heading draws no chevron: {heading}"));
    let title = heading.find("Symbol Dictionary").unwrap_or_else(|| panic!("the heading does not name the block: {heading}"));
    assert!(chevron < title, "the chevron comes after the title: {heading}");
}

#[test]
fn the_chevron_is_the_file_trees() {
    let html = render("Symbol Dictionary", true, None);
    assert!(html.contains(CHEVRON_SVG), "the block does not draw the file tree's chevron:\n{html}");
}

#[test]
fn the_title_is_set_in_the_panel_title_type() {
    let html = render("Symbol Dictionary", true, None);
    let title = tag_with(&html, "panel-title").unwrap_or_else(|| panic!("no element carries the panel-title type:\n{html}"));
    let after = &html[html.find(title).unwrap() + title.len()..];
    assert!(after.starts_with("Symbol Dictionary"), "the panel-title element is not the block's title: {title}{}", &after[..after.len().min(40)]);
}

#[test]
fn blocks_with_different_titles_name_their_bodies_apart() {
    let a = render("Symbol Dictionary", true, None);
    let b = render("Syntax Tree", true, None);
    let id = |html: &str| attr(tag(html, "button").expect("a button heading"), "aria-controls").expect("aria-controls").to_string();
    assert_ne!(id(&a), id(&b), "two blocks control the same body id");
}

#[test]
fn an_explicit_id_names_the_body() {
    let html = render("Symbol Dictionary", true, Some("glossary"));
    let heading = tag(&html, "button").expect("a button heading");
    assert_eq!(attr(heading, "aria-controls"), Some("glossary-body"), "{heading}");
    assert_eq!(attr(heading, "id"), Some("glossary-heading"), "{heading}");
}
