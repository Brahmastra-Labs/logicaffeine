//! `scripts/web/merge-ssg.sh` turns the prerender server's pages into deployable documents.
//!
//! `dx build --ssg` runs the prerender server before it writes the final shell, so the server
//! renders every page through dioxus's built-in fallback template: `<html>` with no `lang`, and no
//! `<title>` of its own — the fork's head writer then emits the page's `<title>` *before*
//! `<!DOCTYPE html>`. Every prerendered page opened in quirks mode with its language unlabelled.
//! merge-ssg.sh already rebuilds each page's head from the final shell; these tests hold it to
//! rebuilding the document prologue too, on a synthetic build laid out the way dx lays one out.
//!
//! The server also marks every dynamic text node for hydration with a `<!--node-idN-->` comment —
//! inside raw-text elements too, where it is content, not a comment. In a `<style>` it turns the
//! first rule's selector into `node-idN-- > …` and the browser drops the rule; `GLOBAL_STYLE`'s
//! first rule is the palette, so every prerendered page painted without its colours. The client
//! never hydrates, so merge-ssg.sh strips the markers there. `scripts/web/verify-ssg.sh` checks the
//! prologue and the markers on the real artifact.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

const LOADER: &str = "/./assets/logicaffeine-web-dxh0123456789abcd.js";

/// The final shell as dx writes it into public/index.html.
fn shell() -> String {
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n    <meta charset=\"UTF-8\" />\n    \
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\" />\n    \
         <title>LOGICAFFEINE | Debug Your Thoughts</title>\n    \
         <link rel=\"stylesheet\" href=\"/assets/style.css\">\n\
         <link rel=\"preload\" as=\"script\" href=\"{LOADER}\" crossorigin></head>\n<body>\n    \
         <div id=\"app\"></div>\n    <div id=\"main\"></div>\n\
         <script type=\"module\" async src=\"{LOADER}\"></script></body>\n</html>\n"
    )
}

/// A page as the prerender server writes it through the fallback template.
fn prerendered(title: &str, route: &str, body: &str) -> String {
    format!(
        "<title>{title}</title><!DOCTYPE html>\n        <html>\n            <head> \
         <meta name=\"title\" content=\"{title}\"/>\
         <link rel=\"canonical\" href=\"https://logicaffeine.com{route}\"/></head>\n            <body>\n                \
         <div id=\"main\"><div id=\"app-root\">{body}</div></div>\n            </body>\n        </html>"
    )
}

const LEARN_TITLE: &str = "Learn First-Order Logic | LOGICAFFEINE";
/// A prerendered body the way the server writes one: a dynamic `<style>` (the palette first),
/// a dynamic `<textarea>` and inline `<script>`, each opening with its hydration marker, and an
/// ordinary marked text node outside them.
const LEARN_BODY: &str = "<style data-node-hydration=\"0\"><!--node-id1-->:root { --bg-gradient-start: #09090b; }\n\
    html, body { background: var(--bg-gradient-start); }</style><h1>Learn Logic</h1>\
    <p><!--node-id2-->Eighteen modules, from syllogisms to modal logic.</p>\
    <textarea data-node-hydration=\"3\"><!--node-id4-->Every man runs.</textarea>\
    <script data-node-hydration=\"5\"><!--node-id6-->window.demo = 1;</script>";

/// A scratch web dir: public/ holding the shell, prerendered/ holding `/` and `/learn`.
fn stage(tag: &str) -> PathBuf {
    let web = std::env::temp_dir().join(format!("ssg_merge_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&web);
    std::fs::create_dir_all(web.join("public")).unwrap();
    std::fs::create_dir_all(web.join("prerendered/learn")).unwrap();
    std::fs::write(web.join("public/index.html"), shell()).unwrap();
    std::fs::write(
        web.join("prerendered/index.html"),
        prerendered("LOGICAFFEINE | Debug Your Thoughts", "/", "<h1>Debug Your Thoughts</h1>"),
    )
    .unwrap();
    std::fs::write(web.join("prerendered/learn/index.html"), prerendered(LEARN_TITLE, "/learn", LEARN_BODY)).unwrap();
    web
}

fn merge(web: &Path) {
    let out = Command::new("bash")
        .arg(repo_root().join("scripts/web/merge-ssg.sh"))
        .arg(web)
        .output()
        .expect("bash runs scripts/web/merge-ssg.sh");
    assert!(
        out.status.success(),
        "merge-ssg.sh failed ({}):\n{}{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn merged_learn(tag: &str) -> String {
    let web = stage(tag);
    merge(&web);
    let page = std::fs::read_to_string(web.join("public/learn/index.html")).unwrap();
    let _ = std::fs::remove_dir_all(&web);
    page
}

fn head(page: &str) -> &str {
    let open = page.find("<head").expect("the merged page has a <head>");
    let open = open + page[open..].find('>').unwrap() + 1;
    let close = page.find("</head>").expect("the merged page closes its <head>");
    &page[open..close]
}

#[test]
fn a_merged_page_opens_in_standards_mode() {
    let page = merged_learn("doctype");
    assert!(
        page.trim_start().starts_with("<!DOCTYPE html>"),
        "a merged page must open with <!DOCTYPE html>, or the browser renders it in quirks mode; it opens with {:?}",
        &page.trim_start()[..page.trim_start().len().min(80)]
    );
}

#[test]
fn a_merged_page_declares_the_shell_language() {
    let page = merged_learn("lang");
    assert!(
        page.contains("<html lang=\"en\">"),
        "a merged page must carry the shell's <html lang=\"en\">, or screen readers are not told its language"
    );
}

#[test]
fn a_merged_page_keeps_its_own_title_once_inside_head() {
    let page = merged_learn("title");
    let titles: Vec<&str> = page.match_indices("<title>").map(|(at, _)| &page[at..]).collect();
    assert_eq!(titles.len(), 1, "a merged page must carry exactly one <title>; found {}", titles.len());
    assert!(
        head(&page).contains(&format!("<title>{LEARN_TITLE}</title>")),
        "the page's own title must sit inside <head> (not the shell's, not before the document)"
    );
}

#[test]
fn merging_twice_changes_nothing() {
    let web = stage("idempotent");
    merge(&web);
    let once = std::fs::read_to_string(web.join("public/learn/index.html")).unwrap();
    let landing_once = std::fs::read_to_string(web.join("public/index.html")).unwrap();
    merge(&web);
    let twice = std::fs::read_to_string(web.join("public/learn/index.html")).unwrap();
    let landing_twice = std::fs::read_to_string(web.join("public/index.html")).unwrap();
    let _ = std::fs::remove_dir_all(&web);
    assert_eq!(once, twice, "a second merge rewrote /learn");
    assert_eq!(landing_once, landing_twice, "a second merge rewrote the landing page");
}

#[test]
fn a_merged_page_keeps_its_body_and_boots_the_app() {
    let page = merged_learn("body");
    assert!(
        page.contains("<h1>Learn Logic</h1>") && page.contains("Eighteen modules, from syllogisms to modal logic."),
        "the prerendered body must survive the merge"
    );
    assert!(head(&page).contains("/assets/style.css"), "the shell's stylesheet must reach the page head");
    assert!(page.contains(&format!("<script type=\"module\" async src=\"{LOADER}\"></script>")), "the wasm loader must reach the page");
    let app = page.find("<div id=\"app\"></div>").expect("the page needs the #app mount point");
    let main = page.find("<div id=\"main\">").unwrap();
    assert!(app < main, "#app must precede #main, where the prerendered copy waits for the takeover");
}

/// Every raw-text element's content in `page`, with the element's name.
fn raw_text(page: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for tag in ["style", "script", "title", "textarea"] {
        let mut rest = page;
        while let Some(at) = rest.find(&format!("<{tag}")) {
            let open_end = at + rest[at..].find('>').unwrap() + 1;
            let close = open_end + rest[open_end..].find(&format!("</{tag}>")).unwrap();
            found.push((tag.to_string(), rest[open_end..close].to_string()));
            rest = &rest[close..];
        }
    }
    found
}

#[test]
fn no_raw_text_element_keeps_a_hydration_marker() {
    let page = merged_learn("markers");
    let marked: Vec<String> = raw_text(&page)
        .into_iter()
        .filter(|(_, text)| text.contains("<!--node-id"))
        .map(|(tag, text)| format!("<{tag}>{}", &text[..text.len().min(40)]))
        .collect();
    assert!(
        marked.is_empty(),
        "a hydration marker inside a raw-text element is content, not a comment — in a <style> it \
         drops the first rule; still marked: {marked:?}"
    );
}

#[test]
fn a_merged_style_opens_with_its_first_rule() {
    let page = merged_learn("palette");
    let (_, palette) = raw_text(&page)
        .into_iter()
        .find(|(tag, text)| tag == "style" && text.contains("--bg-gradient-start: #09090b"))
        .expect("the palette style survives the merge");
    assert!(palette.starts_with(":root {"), "the palette rule must open the style, where the browser parses it: {palette:?}");
}

#[test]
fn markers_outside_raw_text_elements_are_left_alone() {
    let page = merged_learn("outside");
    assert!(
        page.contains("<p><!--node-id2-->Eighteen modules"),
        "a marker in ordinary markup is a real comment and harmless; the merge leaves it"
    );
    assert!(page.contains("Every man runs.</textarea>"), "the textarea keeps its text");
    assert!(page.contains("window.demo = 1;</script>"), "the script keeps its code");
}
