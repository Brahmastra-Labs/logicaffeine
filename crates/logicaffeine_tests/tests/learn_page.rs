//! What the Learn Logic page renders, checked on the native SSR path.
//!
//! The same render the SSG prerender ships: the roadmap, the lesson a first visit opens, and every
//! lesson as the page draws it, read out of the HTML and held against the curriculum. The rules
//! behind the page live in the `learn_state`, `composer` and `progress` unit tests and the content
//! in `learn_lessons.rs`; what only a browser can show (clicks, keys, the caret, storage, a phone's
//! width) lives in scripts/web/test-learn-e2e.mjs.

#![cfg(feature = "web-tests")]

use dioxus::prelude::*;
use dioxus_history::{History, MemoryHistory};
use logicaffeine_web::composer::symbol_buttons;
use logicaffeine_web::content::{ContentEngine, LessonQuestion, Section};
use logicaffeine_web::ui::pages::learn::LessonPanel;
use logicaffeine_web::App;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::OnceLock;

fn learn_html() -> &'static str {
    static HTML: OnceLock<String> = OnceLock::new();
    HTML.get_or_init(|| {
        let mut vdom = VirtualDom::new(App);
        vdom.provide_root_context(Rc::new(MemoryHistory::with_initial_path("/learn")) as Rc<dyn History>);
        vdom.rebuild_in_place();
        dioxus_ssr::render(&vdom)
    })
}

/// One lesson as the page draws it.
fn lesson_html(id: &str) -> String {
    #[derive(Props, Clone, PartialEq)]
    struct HarnessProps {
        id: String,
    }
    #[allow(non_snake_case)]
    fn Harness(props: HarnessProps) -> Element {
        rsx! { LessonPanel { lesson: props.id.clone(), done: false, on_next: move |_| {} } }
    }
    let mut vdom = VirtualDom::new_with_props(Harness, HarnessProps { id: id.to_string() });
    vdom.rebuild_in_place();
    dioxus_ssr::render(&vdom)
}

fn sections() -> Vec<(&'static str, &'static Section)> {
    ContentEngine::shared()
        .eras()
        .iter()
        .flat_map(|e| e.modules.iter())
        .flat_map(|m| m.sections.iter().map(move |s| (m.meta.id.as_str(), s)))
        .collect()
}

/// Every value of `attr="…"` in document order.
fn attr_values<'a>(html: &'a str, attr: &str) -> Vec<&'a str> {
    let needle = format!(" {attr}=\"");
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find(&needle) {
        let after = &rest[i + needle.len()..];
        let end = after.find('"').expect("unterminated attribute");
        out.push(&after[..end]);
        rest = &after[end..];
    }
    out
}

/// Text with tags removed and the entities dioxus-ssr emits decoded; style sheets and scripts are
/// left out. Every tag boundary reads as a space, so the text of two blocks never runs together.
fn text_of(fragment: &str) -> String {
    let mut out = String::new();
    let mut rest = fragment;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        out.push(' ');
        let close = rest[open..].find('>').map_or(rest.len(), |n| open + n + 1);
        let tag = &rest[open..close];
        rest = &rest[close..];
        // A style sheet or a script is not text the page shows.
        for raw in ["style", "script"] {
            if tag.starts_with(&format!("<{raw}")) {
                let end = format!("</{raw}>");
                rest = rest.find(&end).map_or("", |n| &rest[n + end.len()..]);
            }
        }
    }
    out.push_str(rest);
    out.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&#x27;", "'")
}

/// The opening tags carrying `class` among their classes, in document order.
fn tags_with_class<'a>(html: &'a str, class: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(open) = html[i..].find('<').map(|n| n + i) {
        let close = html[open..].find('>').map(|n| n + open).unwrap_or(html.len());
        let tag = &html[open..close];
        if attr_values(tag, "class").iter().any(|c| c.split_whitespace().any(|w| w == class)) {
            out.push(tag);
        }
        i = close.max(open + 1);
    }
    out
}

/// The HTML from the element that opens with `class` to the end of its subtree (by tag depth).
fn element_with_class<'a>(html: &'a str, class: &str) -> Option<&'a str> {
    let tag = *tags_with_class(html, class).first()?;
    let start = html.find(tag)?;
    let name: String = tag[1..].chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
    let (open, close) = (format!("<{name}"), format!("</{name}>"));
    let mut depth = 0usize;
    let mut i = start;
    loop {
        let next_open = html[i..].find(&open).map(|n| n + i);
        let next_close = html[i..].find(&close).map(|n| n + i)?;
        match next_open {
            Some(o) if o < next_close => {
                depth += 1;
                i = o + open.len();
            }
            _ => {
                depth -= 1;
                i = next_close + close.len();
                if depth == 0 {
                    return Some(&html[start..i]);
                }
            }
        }
    }
}

/// `(level, text)` for every heading, in document order.
fn headings(html: &str) -> Vec<(u8, String)> {
    let mut out = Vec::new();
    let bytes = html.as_bytes();
    let mut i = 0;
    while i + 3 < bytes.len() {
        if bytes[i] == b'<' && bytes[i + 1] == b'h' && (b'1'..=b'6').contains(&bytes[i + 2]) && matches!(bytes[i + 3], b' ' | b'>') {
            let level = bytes[i + 2] - b'0';
            let body_start = html[i..].find('>').unwrap() + i + 1;
            let body_end = html[body_start..].find(&format!("</h{level}>")).unwrap() + body_start;
            out.push((level, text_of(&html[body_start..body_end]).split_whitespace().collect::<Vec<_>>().join(" ")));
            i = body_end;
        } else {
            i += 1;
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// The roadmap
// ---------------------------------------------------------------------------------------------

#[test]
fn the_roadmap_lists_every_lesson_in_curriculum_order() {
    let roadmap = element_with_class(learn_html(), "learn-roadmap").expect("the page has a roadmap");
    let shown: Vec<&str> = tags_with_class(roadmap, "roadmap-lesson").iter().flat_map(|t| attr_values(t, "data-lesson")).collect();
    let expected: Vec<&str> = sections().iter().map(|(_, s)| s.id.as_str()).collect();
    assert_eq!(shown, expected);
}

#[test]
fn the_roadmap_names_every_lesson_by_its_title() {
    let roadmap = element_with_class(learn_html(), "learn-roadmap").unwrap();
    let text = text_of(roadmap);
    for (_, section) in sections() {
        assert!(text.contains(&section.title), "the roadmap does not name lesson {} ({:?})", section.id, section.title);
    }
}

#[test]
fn the_roadmap_heads_each_era_once_and_each_module_where_its_link_lands() {
    let html = learn_html();
    let roadmap = element_with_class(html, "learn-roadmap").unwrap();
    let hs = headings(roadmap);
    let ids: HashSet<&str> = attr_values(html, "id").into_iter().collect();
    for era in ContentEngine::shared().eras() {
        let n = hs.iter().filter(|(_, t)| *t == era.meta.title).count();
        assert_eq!(n, 1, "era {:?} is headed {n} times in the roadmap", era.meta.title);
        for module in &era.modules {
            assert!(hs.iter().any(|(_, t)| t.contains(&module.meta.title)), "module {:?} has no heading", module.meta.title);
            // The site nav's drawer links /learn#<module>.
            assert!(ids.contains(module.meta.id.as_str()), "no element carries the id {} that /learn#{} lands on", module.meta.id, module.meta.id);
        }
    }
}

#[test]
fn a_first_visit_opens_the_first_lesson() {
    let html = learn_html();
    let first = sections()[0].1.id.as_str();
    let article = element_with_class(html, "lesson").expect("a lesson is open");
    assert_eq!(attr_values(article, "data-lesson").first().copied(), Some(first));
    let current: Vec<&str> = tags_with_class(html, "roadmap-lesson").into_iter().filter(|t| t.contains("aria-current=\"step\"")).collect();
    assert_eq!(current.len(), 1, "exactly one lesson is marked as the open one: {current:?}");
    assert!(current[0].contains(&format!("data-lesson=\"{first}\"")), "{current:?}");
}

#[test]
fn the_page_counts_the_lessons_done() {
    let total = sections().len();
    assert!(text_of(learn_html()).contains(&format!("0 of {total} lessons done")), "no lesson count");
}

#[test]
fn the_page_keeps_no_score() {
    let words = ["xp", "level", "levels", "streak", "streaks", "combo", "combos", "achievement", "achievements", "badge", "badges", "trophy"];
    let mut v = Vec::new();
    let mut pages = vec![("/learn".to_string(), text_of(learn_html()))];
    pages.extend(sections().iter().map(|(_, s)| (format!("lesson {}", s.id), text_of(&lesson_html(&s.id)))));
    for (at, text) in &pages {
        for w in text.split(|c: char| !c.is_alphanumeric()).map(str::to_lowercase) {
            if words.contains(&w.as_str()) {
                v.push(format!("{at}: {w:?}"));
            }
        }
    }
    v.dedup();
    assert!(v.is_empty(), "the Learn page keeps score: {v:?}");
}

#[test]
fn the_site_nav_offers_no_profile() {
    let html = learn_html();
    assert!(!attr_values(html, "href").iter().any(|h| h.starts_with("/profile")), "a link to /profile");
    assert!(!text_of(html).contains("Profile") && !html.contains("Your Profile"), "the nav still names a profile");
}

// ---------------------------------------------------------------------------------------------
// One lesson
// ---------------------------------------------------------------------------------------------

#[test]
fn every_lesson_shows_two_examples_and_one_question() {
    let mut v = Vec::new();
    for (_, section) in sections() {
        let html = lesson_html(&section.id);
        let examples = tags_with_class(&html, "lesson-example").len();
        if examples != 2 {
            v.push(format!("{}: {examples} examples on the main path", section.id));
        }
        let questions = tags_with_class(&html, "lesson-question").len();
        if questions != 1 {
            v.push(format!("{}: {questions} questions", section.id));
        }
        if !text_of(&html).contains(&section.title) {
            v.push(format!("{}: the title {:?} is not shown", section.id, section.title));
        }
    }
    assert!(v.is_empty(), "\n{}\n", v.join("\n"));
}

#[test]
fn a_typed_question_has_a_field_and_every_symbol_button() {
    let mut v = Vec::new();
    for (_, section) in sections() {
        let html = lesson_html(&section.id);
        let typed = matches!(section.question, Some(LessonQuestion::Translate { .. }));
        let pads = tags_with_class(&html, "symbol-pad").len();
        let fields = tags_with_class(&html, "lesson-input").len();
        if typed {
            if (pads, fields) != (1, 1) {
                v.push(format!("{}: a typed question with {fields} fields and {pads} symbol pads", section.id));
            }
            let keys: Vec<&str> = tags_with_class(&html, "symbol-key").into_iter().flat_map(|t| attr_values(t, "data-symbol")).collect();
            let expected: Vec<&str> = symbol_buttons().iter().map(|b| b.symbol).collect();
            if keys != expected {
                v.push(format!("{}: the pad offers {keys:?}", section.id));
            }
        } else if pads + fields > 0 {
            v.push(format!("{}: a choice question with a text field or symbol pad", section.id));
        }
    }
    assert!(v.is_empty(), "\n{}\n", v.join("\n"));
}

#[test]
fn a_choice_question_offers_each_option_as_a_button() {
    let mut v = Vec::new();
    for (_, section) in sections() {
        let Some(LessonQuestion::Choice { options, .. }) = &section.question else { continue };
        let html = lesson_html(&section.id);
        let group = element_with_class(&html, "lesson-options").unwrap_or("");
        let buttons = group.matches("<button").count();
        if buttons != options.len() {
            v.push(format!("{}: {buttons} option buttons for {} options", section.id, options.len()));
        }
        for option in options {
            if !text_of(group).contains(option.as_str()) {
                v.push(format!("{}: option {option:?} is not shown", section.id));
            }
        }
    }
    assert!(v.is_empty(), "\n{}\n", v.join("\n"));
}

#[test]
fn symbol_buttons_are_named_buttons_that_keep_the_learner_typing() {
    let typed = sections().into_iter().find(|(_, s)| matches!(s.question, Some(LessonQuestion::Translate { .. }))).expect("a typed lesson").1;
    let html = lesson_html(&typed.id);
    for (tag, button) in tags_with_class(&html, "symbol-key").into_iter().zip(symbol_buttons()) {
        assert!(tag.starts_with("<button") && tag.contains("type=\"button\""), "{tag}");
        assert_eq!(attr_values(tag, "aria-label").first().copied(), Some(button.label().as_str()), "{tag}");
        assert_eq!(attr_values(tag, "title").first().copied(), Some(button.meaning.as_str()), "{tag}");
        assert!(tag.contains("aria-controls=\"lesson-answer\""), "a button names the field it types into: {tag}");
    }
}

#[test]
fn more_examples_and_more_practice_wait_to_be_asked_for() {
    let engine = ContentEngine::shared();
    let mut v = Vec::new();
    for (module_id, section) in sections() {
        let html = lesson_html(&section.id);
        let extra = section.examples.len().saturating_sub(2);
        let more = tags_with_class(&html, "lesson-more-examples");
        match (extra, more.as_slice()) {
            (0, []) => {}
            (0, _) => v.push(format!("{}: offers more examples it does not have", section.id)),
            (n, [toggle]) => {
                if !toggle.contains("aria-expanded=\"false\"") {
                    v.push(format!("{}: more examples open before they are asked for", section.id));
                }
                if !text_of(&html).contains(&format!("More examples ({n})")) {
                    v.push(format!("{}: the toggle does not count its {n} examples", section.id));
                }
            }
            (_, found) => v.push(format!("{}: {} more-examples toggles", section.id, found.len())),
        }
        let module = engine.eras().iter().flat_map(|e| e.modules.iter()).find(|m| m.meta.id == module_id).unwrap();
        let practice = module.exercises.iter().filter(|e| e.section.as_deref() == Some(section.id.as_str())).count();
        let toggles = tags_with_class(&html, "lesson-more-practice");
        if practice == 0 && !toggles.is_empty() {
            v.push(format!("{}: offers practice it does not have", section.id));
        }
        if practice > 0 {
            if toggles.len() != 1 || !toggles[0].contains("aria-expanded=\"false\"") {
                v.push(format!("{}: more practice is not one closed toggle: {toggles:?}", section.id));
            }
            if !text_of(&html).contains(&format!("More practice ({practice})")) {
                v.push(format!("{}: the toggle does not count its {practice} questions", section.id));
            }
        }
        if !tags_with_class(&html, "practice-card").is_empty() {
            v.push(format!("{}: a practice question shows before practice is asked for", section.id));
        }
    }
    assert!(v.is_empty(), "\n{}\n", v.join("\n"));
}

#[test]
fn the_next_lesson_waits_for_an_answer() {
    for (_, section) in sections() {
        let html = lesson_html(&section.id);
        assert!(tags_with_class(&html, "lesson-next").is_empty(), "{}: Next shows before the question is answered", section.id);
    }
}

#[test]
fn every_lesson_reads_without_an_em_dash_or_a_semicolon() {
    let mut v = Vec::new();
    for (_, section) in sections() {
        let text = text_of(&lesson_html(&section.id));
        for bad in ['\u{2014}', ';'] {
            if text.contains(bad) {
                v.push(format!("{}: {bad:?}", section.id));
            }
        }
    }
    assert!(v.is_empty(), "\n{}\n", v.join("\n"));
}

// ---------------------------------------------------------------------------------------------
// The page as a document
// ---------------------------------------------------------------------------------------------

#[test]
fn every_aria_reference_resolves() {
    let mut v = Vec::new();
    let mut pages = vec![("/learn".to_string(), learn_html().to_string())];
    pages.extend(sections().iter().map(|(_, s)| (format!("lesson {}", s.id), lesson_html(&s.id))));
    for (at, html) in &pages {
        let ids: HashSet<&str> = attr_values(html, "id").into_iter().collect();
        for attr in ["aria-labelledby", "aria-controls", "aria-describedby"] {
            for value in attr_values(html, attr) {
                for id in value.split_whitespace() {
                    if !ids.contains(id) {
                        v.push(format!("{at}: {attr}=\"{id}\""));
                    }
                }
            }
        }
    }
    v.dedup();
    assert!(v.is_empty(), "ARIA references with no target: {v:?}");
}

#[test]
fn ids_are_unique_on_the_page() {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for id in attr_values(learn_html(), "id") {
        *seen.entry(id).or_default() += 1;
    }
    let dups: Vec<_> = seen.into_iter().filter(|(_, n)| *n > 1).collect();
    assert!(dups.is_empty(), "duplicate ids: {dups:?}");
}

#[test]
fn no_heading_repeats_the_one_before_it() {
    let hs = headings(learn_html());
    let repeats: Vec<_> = hs.windows(2).filter(|w| w[0].1 == w[1].1).map(|w| w[1].1.clone()).collect();
    assert!(repeats.is_empty(), "headings rendered twice in a row: {repeats:?}");
}

#[test]
fn the_page_has_one_top_heading() {
    let ones: Vec<_> = headings(learn_html()).into_iter().filter(|(l, _)| *l == 1).collect();
    assert_eq!(ones, [(1, "Learn Logic".to_string())]);
}

#[test]
fn learn_styles_draw_accents_from_the_site_palette() {
    // The site's palette is defined once, as CSS tokens in the global stylesheet; a page that
    // hardcodes its own copy of those colours drifts the moment the palette changes.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/logicaffeine_web");
    let banned = [
        "#60a5fa", "#a78bfa", "#fbbf24", "#070a12", "#0b1022", "#060814", "96,165,250", "96, 165, 250", "167,139,250",
        "167, 139, 250", "251, 191, 36", "251,191,36",
    ];
    let mut hits = Vec::new();
    for file in ["src/ui/pages/learn.rs", "src/ui/components/learn_roadmap.rs", "src/ui/components/symbol_pad.rs"] {
        let text = std::fs::read_to_string(dir.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
        for (n, line) in text.lines().enumerate() {
            if banned.iter().any(|b| line.contains(b)) {
                hits.push(format!("{file}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    assert!(hits.is_empty(), "\nhardcoded theme colours:\n{}\n", hits.join("\n"));
}
