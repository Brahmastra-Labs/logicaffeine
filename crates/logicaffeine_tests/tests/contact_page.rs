//! The contact page as it is prerendered, held to the truth.
//!
//! The page says LOGOS is open source under MIT and nothing more about licensing; every capability
//! it claims names the suite that proves it, and a list of claims the product does not back can
//! never reach the page. The page is called Contact wherever it is linked, `/pricing` (its old
//! address) redirects here, and the form asks for an email (required) and a phone (optional) with
//! nothing hidden from the reader but the honeypot. Where things are painted and how the form
//! behaves in a browser is `scripts/web/test-site-e2e.mjs`.

#![cfg(feature = "web-tests")]

use dioxus::prelude::*;
use dioxus_history::{History, MemoryHistory};
use logicaffeine_web::ui::pages::guide::content::SECTIONS;
use logicaffeine_web::ui::pages::contact::content::{self, Evidence};
use logicaffeine_web::ui::pages::contact::lead;
use logicaffeine_web::ui::router::Route;
use logicaffeine_web::ui::seo::pages as seo_pages;
use logicaffeine_web::App;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::OnceLock;

// ---------------------------------------------------------------------------------------------
// Rendering and reading the page
// ---------------------------------------------------------------------------------------------

fn render_route(path: &str) -> String {
    let mut vdom = VirtualDom::new(App);
    vdom.provide_root_context(Rc::new(MemoryHistory::with_initial_path(path)) as Rc<dyn History>);
    vdom.rebuild_in_place();
    dioxus_ssr::render(&vdom)
}

fn contact_page() -> &'static str {
    static HTML: OnceLock<String> = OnceLock::new();
    HTML.get_or_init(|| render_route("/contact"))
}

fn landing() -> &'static str {
    static HTML: OnceLock<String> = OnceLock::new();
    HTML.get_or_init(|| render_route("/"))
}

/// Every prerendered route with its rendered HTML.
fn every_page() -> &'static [(String, String)] {
    static PAGES: OnceLock<Vec<(String, String)>> = OnceLock::new();
    PAGES.get_or_init(|| {
        logicaffeine_web::sitemap::prerender_routes()
            .into_iter()
            .map(|path| {
                let html = render_route(&path);
                (path, html)
            })
            .collect()
    })
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn repo_file(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Text as the prerender writes it: the five characters HTML escapes, as numeric references.
fn escaped(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '"' => "&#34;".to_string(),
            '&' => "&#38;".to_string(),
            '\'' => "&#39;".to_string(),
            '<' => "&#60;".to_string(),
            '>' => "&#62;".to_string(),
            c => c.to_string(),
        })
        .collect()
}

/// Character references read back to the characters they stand for.
fn decode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        let Some(semi) = tail.find(';').filter(|&i| i <= 10) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let name = &tail[1..semi];
        let decoded = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            _ if name.starts_with("#x") || name.starts_with("#X") => u32::from_str_radix(&name[2..], 16).ok().and_then(char::from_u32),
            _ if name.starts_with('#') => name[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &tail[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The markup with `<script>` and `<style>` elements removed.
fn without_raw_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    loop {
        let next = ["<script", "<style"].iter().filter_map(|open| rest.find(open).map(|i| (i, *open))).min();
        let Some((at, open)) = next else { break };
        out.push_str(&rest[..at]);
        let close = if open == "<script" { "</script>" } else { "</style>" };
        rest = match rest[at..].find(close) {
            Some(end) => &rest[at + end + close.len()..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// Elements that sit inside a line of text: a browser runs their text on with the text around
/// them, where every other element starts a new run.
const INLINE_TAGS: &[&str] = &["a", "abbr", "b", "cite", "code", "em", "i", "kbd", "q", "small", "span", "strong", "sub", "sup"];

/// What a reader sees: tags, scripts and styles removed, references decoded, whitespace collapsed.
fn visible_text(html: &str) -> String {
    let markup = without_raw_text(html);
    let mut text = String::with_capacity(markup.len());
    let mut rest = markup.as_str();
    while let Some(lt) = rest.find('<') {
        text.push_str(&rest[..lt]);
        let tag = &rest[lt + 1..];
        let name: String = tag.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        if !INLINE_TAGS.contains(&name.to_ascii_lowercase().as_str()) {
            text.push(' ');
        }
        rest = match tag.find('>') {
            Some(gt) => &tag[gt + 1..],
            None => "",
        };
    }
    text.push_str(rest);
    decode(&text).replace('\u{a0}', " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every JSON-LD block the markup carries, parsed.
fn json_ld(html: &str) -> Vec<serde_json::Value> {
    html.split("<script type=\"application/ld+json\">")
        .skip(1)
        .map(|tail| {
            let json = &tail[..tail.find("</script>").expect("a closed JSON-LD script")];
            serde_json::from_str(json).unwrap_or_else(|e| panic!("a JSON-LD block does not parse ({e}):\n{json}"))
        })
        .collect()
}

/// The markup from the element carrying `id="{id}"` up to the next top-level page section.
fn section<'a>(html: &'a str, id: &str) -> &'a str {
    let needle = format!("id=\"{id}\"");
    let start = html.find(&needle).unwrap_or_else(|| panic!("the page renders no element with {needle}"));
    let start = html[..start].rfind('<').expect("the id sits inside a tag");
    let end = html[start + 1..]
        .find("<section")
        .map(|i| start + 1 + i)
        .or_else(|| html[start..].find("<footer").map(|i| start + i))
        .unwrap_or(html.len());
    &html[start..end]
}

/// The attributes of one start tag (`<input type="email" required=true …>`), by name.
fn attributes(tag: &str) -> BTreeMap<String, String> {
    let body = tag.trim_start_matches('<');
    let body = body.trim_end_matches('>').trim_end_matches('/');
    let mut attrs = BTreeMap::new();
    let mut chars = body.char_indices().peekable();
    // Skip the tag name.
    while let Some(&(_, c)) = chars.peek() {
        if c.is_whitespace() {
            break;
        }
        chars.next();
    }
    loop {
        while chars.peek().is_some_and(|&(_, c)| c.is_whitespace()) {
            chars.next();
        }
        let Some(&(name_start, _)) = chars.peek() else { break };
        let mut name_end = body.len();
        while let Some(&(i, c)) = chars.peek() {
            if c == '=' || c.is_whitespace() {
                name_end = i;
                break;
            }
            chars.next();
        }
        let name = body[name_start..name_end.min(body.len())].to_string();
        if chars.peek().is_some_and(|&(_, c)| c == '=') {
            chars.next();
            let value = match chars.peek().map(|&(_, c)| c) {
                Some(q @ ('"' | '\'')) => {
                    chars.next();
                    let mut v = String::new();
                    for (_, c) in chars.by_ref() {
                        if c == q {
                            break;
                        }
                        v.push(c);
                    }
                    v
                }
                _ => {
                    let mut v = String::new();
                    while let Some(&(_, c)) = chars.peek() {
                        if c.is_whitespace() {
                            break;
                        }
                        v.push(c);
                        chars.next();
                    }
                    v
                }
            };
            attrs.insert(name, decode(&value));
        } else if !name.is_empty() {
            attrs.insert(name, String::new());
        }
    }
    attrs
}

/// Every start tag named `name` in the markup, as its attributes.
fn tags(html: &str, name: &str) -> Vec<BTreeMap<String, String>> {
    let open = format!("<{name}");
    let mut found = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find(&open) {
        let tail = &rest[at..];
        let after = tail[open.len()..].chars().next();
        let end = tail.find('>').expect("a closed start tag");
        if matches!(after, Some(c) if c.is_whitespace() || c == '>' || c == '/') {
            found.push(attributes(&tail[..=end]));
        }
        rest = &tail[end + 1..];
    }
    found
}

/// Every link in the markup: its attributes and the text it shows.
fn links(html: &str) -> Vec<(BTreeMap<String, String>, String)> {
    let mut found = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find("<a") {
        let tail = &rest[at..];
        if !matches!(tail[2..].chars().next(), Some(c) if c.is_whitespace() || c == '>') {
            rest = &tail[2..];
            continue;
        }
        let end = tail.find('>').expect("a closed start tag");
        let attrs = attributes(&tail[..=end]);
        let close = tail.find("</a>").expect("a closed link");
        found.push((attrs, visible_text(&tail[end + 1..close])));
        rest = &tail[close + 4..];
    }
    found
}

/// A boolean attribute that is present and not set false.
fn is_set(attrs: &BTreeMap<String, String>, name: &str) -> bool {
    attrs.get(name).is_some_and(|v| v != "false")
}

/// Every mailbox named in the text: `local@domain.tld` runs.
fn mailboxes(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let is_local = |b: u8| b.is_ascii_alphanumeric() || b"._%+-".contains(&b);
    let is_domain = |b: u8| b.is_ascii_alphanumeric() || b".-".contains(&b);
    let mut found = Vec::new();
    for (at, _) in text.match_indices('@') {
        let mut start = at;
        while start > 0 && is_local(bytes[start - 1]) {
            start -= 1;
        }
        let mut end = at + 1;
        while end < bytes.len() && is_domain(bytes[end]) {
            end += 1;
        }
        let domain = text[at + 1..end].trim_end_matches('.');
        if start < at && domain.contains('.') && domain.rsplit('.').next().is_some_and(|tld| tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic())) {
            found.push(format!("{}@{}", &text[start..at], domain));
        }
    }
    found
}

// ---------------------------------------------------------------------------------------------
// The artifacts, read independently of the page's own readers
// ---------------------------------------------------------------------------------------------






// ---------------------------------------------------------------------------------------------
// 1. Truth first: the license, the claims, the numbers
// ---------------------------------------------------------------------------------------------






/// Claims the product does not back, each with why. Matched without regard to case against
/// everything the page publishes: its text, its structured data, its title and description.
const BANNED_CLAIMS: &[(&str, &str)] = &[
    ("via LLVM", "LOGOS emits Rust source, which rustc compiles; the claim ignores the other tiers"),
    ("Futamura", "the SVA synthesis is fol_to_sva; the Futamura piece is a cached generator"),
    ("GossipSub", "libp2p is feature-gated; the protocol name is jargon the buyer cannot use"),
    ("capability-based", "access control is ## Policy predicates and Check that guards"),
    ("garbage collector", "runtime values are reference-counted in places"),
    ("proves the emitted Rust", "translation validation is experimental and source-side only"),
    ("marketplace", "the VS Code extension ships as a VSIX; the marketplace listing is disabled"),
    ("2.6×", "the 2.62× headline counts five algorithm collapses"),
    ("2.62", "the 2.62× headline counts five algorithm collapses"),
    ("largo verify", "it verified nothing in the released 0.10.1 (plan D2)"),
    ("Z3 SMT", "Z3 verification is off the page until a fixed release ships (plan D2)"),
    ("static verification", "Z3 verification is off the page until a fixed release ships (plan D2)"),
    ("zero interpreter overhead", "the interpreter is one of five tiers"),
    ("tutor prompts", "there is no such feature"),
    ("all of it is public", "the public repository is an export of the tree; confidential parts stay private"),
    ("all of the source", "the public repository is an export of the tree; confidential parts stay private"),
    ("entire source", "the public repository is an export of the tree; confidential parts stay private"),
    ("whole source", "the public repository is an export of the tree; confidential parts stay private"),
    ("no coding", "a person still reads the program; that is the point of the page"),
    ("no code", "a person still reads the program; that is the point of the page"),
    ("bug-free", "no test proves a program free of bugs"),
    ("largo mcp", "the MCP server is on main only, not in the installable 0.10.1"),
    ("agent pack", "the agent pack is on main only, not in the installable 0.10.1"),
];

/// Reply-time promises (plan D3): the team cannot always answer within a fixed window.
const REPLY_TIME_PROMISES: &[&str] = &[
    "business day",
    "24 hours",
    "48 hours",
    "24/7",
    "within the hour",
    "within an hour",
    "within one hour",
    "within a day",
    "within 24",
    "within 48",
    "same day",
    "same-day",
    "response time",
    "response-time",
    "reply time",
    "respond within",
    "reply within",
    "get back to you within",
    "hear back within",
    "service level",
    "service-level",
];

/// The contact page's own content: its `<main>`, without the site's nav and footer. (The mobile
/// nav lists every Syntax Guide section by title on every page; a guide's title is not a claim
/// this page makes.)
fn page_main() -> &'static str {
    let html = contact_page();
    let start = html.find("<main class=\"contact-main\"").expect("the contact page renders its main content");
    let end = start + html[start..].find("</main>").expect("a closed main");
    &html[start..end]
}

/// Everything the contact page publishes, lowercased: what its content says, its structured data,
/// and the title and description its head carries.
fn published_text() -> String {
    let html = contact_page();
    let data: Vec<String> = json_ld(html).iter().map(|v| v.to_string()).collect();
    format!(
        "{}\n{}\n{}\n{}",
        visible_text(page_main()),
        data.join("\n"),
        seo_pages::CONTACT.title,
        seo_pages::CONTACT.description
    )
    .to_lowercase()
}

#[test]
fn no_unbacked_claim_reaches_the_page() {
    let text = published_text();
    let found: Vec<String> = BANNED_CLAIMS
        .iter()
        .filter(|(claim, _)| text.contains(&claim.to_lowercase()))
        .map(|(claim, why)| format!("{claim:?}: {why}"))
        .collect();
    assert!(found.is_empty(), "the contact page makes claims the product does not back:\n{}", found.join("\n"));
    for gone in ["commercial", "logic service", "business source", "busl", "license key", "pricing", "price"] {
        assert!(!text.contains(gone), "the contact page still says {gone:?}");
    }
}

#[test]
fn no_reply_time_is_promised() {
    let text = published_text();
    let mut found: Vec<&str> = REPLY_TIME_PROMISES.iter().copied().filter(|p| text.contains(*p)).collect();
    if text.split(|c: char| !c.is_ascii_alphanumeric()).any(|w| w == "sla" || w == "slas") {
        found.push("SLA");
    }
    assert!(found.is_empty(), "the contact page promises a reply time (plan D3 says it never does): {found:?}");
}

#[test]
fn no_price_appears_on_the_page() {
    let text = published_text();
    let currency_amount = text
        .char_indices()
        .filter(|(_, c)| matches!(c, '$' | '€' | '£'))
        .find(|(i, c)| text[i + c.len_utf8()..].chars().next().is_some_and(|d| d.is_ascii_digit()));
    assert!(currency_amount.is_none(), "the contact page shows a price: {:?}", currency_amount.map(|(i, _)| &text[i..(i + 24).min(text.len())]));
    for rate in ["/mo", "per month", "per seat", "per year", "/year"] {
        assert!(!text.contains(rate), "the contact page shows a rate ({rate:?})");
    }
}

#[test]
fn every_capability_claim_names_a_suite_that_exists() {
    let tests = repo_root().join("crates/logicaffeine_tests/tests");
    let claims = content::every_claim();
    assert!(!claims.is_empty(), "the content lost its claims");
    for (text, evidence) in claims {
        let Evidence::Suites(suites) = evidence;
        assert!(!suites.is_empty(), "{text:?} names no suite");
        for suite in *suites {
            assert!(tests.join(format!("{suite}.rs")).is_file(), "{text:?} cites the suite {suite}, which does not exist");
        }
    }
}


#[test]
fn every_claim_is_on_the_page() {
    let html = contact_page();
    for (text, _) in content::every_claim() {
        assert!(html.contains(&escaped(&text)), "the page does not show the claim {text:?}");
    }
}




// ---------------------------------------------------------------------------------------------
// 2. The page is called Contact
// ---------------------------------------------------------------------------------------------

/// The text of every link to `/contact` in the markup whose class list holds `class`.
fn contact_link_labels(html: &str, class: &str) -> Vec<String> {
    links(html)
        .into_iter()
        .filter(|(attrs, _)| attrs.get("href").is_some_and(|h| h == "/contact" || h.starts_with("/contact#")))
        .filter(|(attrs, _)| attrs.get("class").is_some_and(|c| c.split_whitespace().any(|w| w == class)))
        .map(|(_, text)| text)
        .collect()
}

#[test]
fn the_nav_and_footer_call_the_page_contact_page() {
    for path in ["/", "/contact", "/roadmap"] {
        let html = render_route(path);
        for class in ["main-nav-link", "mobile-nav-link", "footer-link"] {
            let labels = contact_link_labels(&html, class);
            assert_eq!(labels, vec!["Contact".to_string()], "on {path} the {class} to /contact reads {labels:?}");
        }
    }
}


#[test]
fn the_landing_offers_contact() {
    let to_contact: Vec<(String, String)> = links(landing())
        .into_iter()
        .filter_map(|(attrs, text)| attrs.get("href").filter(|h| h.starts_with("/contact")).map(|h| (h.clone(), text)))
        .filter(|(_, text)| !text.is_empty())
        .collect();
    assert!(to_contact.contains(&("/contact".to_string(), "Contact".to_string())), "the landing hero does not offer Contact: {to_contact:?}");
    assert!(!landing().contains("/pricing"), "the landing still links the old pricing address");
}

#[test]
fn the_landing_faq_gives_the_page_s_free_answer() {
    let html = landing();
    let answer = content::free_answer();
    assert!(answer.contains("MIT License"), "the free answer does not name the license: {answer:?}");
    assert!(answer.starts_with("Yes."), "the free answer does not begin with yes: {answer:?}");
    for (claim, why) in BANNED_CLAIMS {
        assert!(!answer.to_lowercase().contains(&claim.to_lowercase()), "the free answer says {claim:?}: {why}");
    }
    let faq = json_ld(html).into_iter().find(|v| v["@type"] == "FAQPage").expect("the landing carries an FAQPage");
    let free = faq["mainEntity"].as_array().unwrap().iter().find(|q| q["name"] == "Is it really free?").expect("the landing's FAQ asks whether it is free");
    assert_eq!(free["acceptedAnswer"]["text"].as_str(), Some(answer.as_str()), "the landing's structured data answers differently");
    assert!(html.contains(&escaped(&answer)), "the landing's visible FAQ answers differently");
}

#[test]
fn the_brand_subtitle_and_breadcrumb_say_contact_page() {
    let html = contact_page();
    assert!(html.contains("<span class=\"main-nav-brand-subtitle\">Contact</span>"), "the nav's subtitle on /contact is not Contact");
    let crumbs = json_ld(html).into_iter().find(|v| v["@type"] == "BreadcrumbList").expect("a BreadcrumbList");
    let items = crumbs["itemListElement"].as_array().unwrap();
    let names: Vec<&str> = items.iter().map(|i| i["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Home", "Contact"], "the breadcrumb reads {names:?}");
    assert_eq!(items.last().unwrap()["item"], "https://logicaffeine.com/contact");
}

#[test]
fn the_title_and_description_say_contact_page() {
    assert!(seo_pages::CONTACT.title.starts_with("Contact"), "the title is {:?}", seo_pages::CONTACT.title);
    assert_eq!(seo_pages::CONTACT.canonical_path, "/contact");
    let description = seo_pages::CONTACT.description;
    assert!(description.contains("MIT License"), "the description does not name the license: {description:?}");
    assert!(!description.to_lowercase().contains("commercial"), "the description still speaks of commercial licensing: {description:?}");
}

#[test]
fn the_structured_data_is_organization_faq_and_breadcrumb() {
    let mut types: Vec<String> = json_ld(contact_page()).iter().map(|v| v["@type"].as_str().unwrap_or("?").to_string()).collect();
    types.sort();
    assert_eq!(types, ["BreadcrumbList", "FAQPage", "Organization"], "the contact page's structured data");
}

#[test]
fn the_organization_names_its_sales_contact() {
    let org = json_ld(contact_page()).into_iter().find(|v| v["@type"] == "Organization").expect("an Organization");
    let point = &org["contactPoint"];
    assert_eq!(point["@type"], "ContactPoint");
    assert_eq!(point["contactType"], "sales");
    assert_eq!(point["email"], content::TEAM_EMAIL);
    assert_eq!(point["url"], "https://logicaffeine.com/contact#contact");
}

#[test]
fn no_page_carries_a_contact_page_schema() {
    for (path, html) in every_page() {
        for block in json_ld(html) {
            assert_ne!(block["@type"], "ContactPage", "{path} still carries a ContactPage");
        }
    }
}

/// The visible FAQ, as (question, answer) pairs in page order.
fn visible_faq() -> Vec<(String, String)> {
    let faq = section(contact_page(), "faq");
    let questions: Vec<String> = faq.split("<dt").skip(1).map(|t| visible_text(&t[t.find('>').unwrap() + 1..t.find("</dt>").unwrap()])).collect();
    let answers: Vec<String> = faq.split("<dd").skip(1).map(|t| visible_text(&t[t.find('>').unwrap() + 1..t.find("</dd>").unwrap()])).collect();
    assert_eq!(questions.len(), answers.len(), "the FAQ has {} questions and {} answers", questions.len(), answers.len());
    questions.into_iter().zip(answers).collect()
}

#[test]
fn the_faq_structured_data_is_the_visible_faq() {
    let faq = json_ld(contact_page()).into_iter().find(|v| v["@type"] == "FAQPage").expect("an FAQPage");
    let data: Vec<(String, String)> = faq["mainEntity"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| (q["name"].as_str().unwrap().to_string(), q["acceptedAnswer"]["text"].as_str().unwrap().to_string()))
        .collect();
    let shown = visible_faq();
    assert_eq!(data, shown, "the FAQPage structured data and the visible FAQ disagree");
    let source: Vec<(String, String)> = content::faq().iter().map(|f| (f.question.to_string(), f.answer.clone())).collect();
    assert_eq!(source, shown, "the visible FAQ is not the FAQ data");
}

/// The four questions, in order (the user, 2026-11-01: open source, under MIT).
#[test]
fn the_faq_asks_four_questions() {
    let questions: Vec<String> = visible_faq().into_iter().map(|(q, _)| q).collect();
    assert_eq!(
        questions,
        ["Is LOGOS free?", "What is the license?", "Where is the source?", "How do I reach the team?"],
        "the FAQ's questions"
    );
    assert!(links(section(contact_page(), "faq")).iter().any(|(a, t)| t == "LICENSE.md" && a.get("href").is_some_and(|h| h.ends_with("/LICENSE.md"))), "the FAQ does not link LICENSE.md");
}

/// The marker the prerender smoke tests look for (plan D7): one to two times in the page, head included.
#[test]
fn the_page_marker_appears_twice() {
    let marker = content::CONTACT_HEADING;
    assert_eq!(marker, "Contact the team");
    let n = contact_page().matches(marker).count();
    assert_eq!(n, 2, "{marker:?} appears {n} times on /contact; the hero's call and the form's heading carry it");
    assert!(!seo_pages::CONTACT.title.contains(marker) && !seo_pages::CONTACT.description.contains(marker), "the head would repeat the marker in title and meta tags");
    let hero = section(contact_page(), "top");
    assert_eq!(hero.matches(marker).count(), 1, "the hero's primary call does not read {marker:?}");
    assert_eq!(section(contact_page(), "contact").matches(marker).count(), 1, "the form's heading does not read {marker:?}");
}

// ---------------------------------------------------------------------------------------------
// 3. /pricing, and the form's anchor
// ---------------------------------------------------------------------------------------------

#[test]
fn pricing_renders_the_contact_page() {
    let html = render_route("/pricing");
    assert!(html.contains("Contact the Team"), "/pricing does not redirect to the contact page");
    assert!(html.contains("id=\"contact\""), "/pricing renders no form section");
}

#[test]
fn every_call_to_the_form_links_its_anchor() {
    let calls: Vec<String> = links(contact_page())
        .into_iter()
        .filter(|(_, text)| text == content::CONTACT_HEADING)
        .map(|(attrs, _)| attrs.get("href").cloned().unwrap_or_default())
        .collect();
    assert!(!calls.is_empty(), "the page never calls to the form: {calls:?}");
    for href in calls {
        assert_eq!(href, "/contact#contact", "a call to the form links {href}");
    }
}

// ---------------------------------------------------------------------------------------------
// 4. The page's parts
// ---------------------------------------------------------------------------------------------

#[test]
fn the_hero_says_what_logos_is() {
    let hero = section(contact_page(), "top");
    let h1s: Vec<String> = hero.split("<h1").skip(1).map(|t| visible_text(&t[t.find('>').unwrap() + 1..t.find("</h1>").unwrap()])).collect();
    assert_eq!(h1s, ["Contact the Team"], "the hero's headline");
    assert_eq!(contact_page().matches("<h1").count(), 1, "the page has more than one h1");
    let text = visible_text(hero);
    assert!(text.contains(content::OPEN_SOURCE_LINE), "the hero does not say LOGOS is open source under MIT: {text}");
    let studio_href = Route::Studio { file: None }.to_string();
    let studio = links(hero).into_iter().any(|(attrs, _)| attrs.get("href").is_some_and(|h| *h == studio_href));
    assert!(studio, "the hero does not offer the browser");
}

#[test]
fn the_install_commands_are_the_guides() {
    let hero = section(contact_page(), "top");
    for command in [content::INSTALL_UNIX, content::INSTALL_WINDOWS] {
        assert!(hero.contains(&escaped(command)), "the hero does not show {command:?}");
        let guide = SECTIONS.iter().find(|s| s.id == "getting-started").expect("the guide's getting-started section");
        assert!(guide.content.contains(command), "the guide's install instructions no longer read {command:?}");
    }
    assert!(content::INSTALL_UNIX.ends_with("https://logicaffeine.com/install.sh | sh"));
    assert!(content::INSTALL_WINDOWS.contains("https://logicaffeine.com/install.ps1"));
    for script in ["install.sh", "install.ps1"] {
        assert!(repo_root().join("apps/logicaffeine_web/public").join(script).is_file(), "the site does not serve {script}");
    }
    let copies = tags(hero, "button").into_iter().filter(|b| b.get("aria-label").is_some_and(|l| l.starts_with("Copy "))).count();
    assert_eq!(copies, 2, "the hero has {copies} copy buttons for two commands");
}




/// The text of every `<tag>` element in the markup, in order.
fn element_texts(html: &str, tag: &str) -> Vec<String> {
    let close = format!("</{tag}>");
    html.split(&format!("<{tag}"))
        .skip(1)
        .filter(|t| t.starts_with('>') || t.starts_with(' '))
        .map(|t| visible_text(&t[t.find('>').unwrap() + 1..t.find(&close).unwrap()]))
        .collect()
}

#[test]
fn the_sections_are_headed_by_short_nouns() {
    assert_eq!(
        element_texts(page_main(), "h2"),
        ["Install in one line", content::CONTACT_HEADING, "FAQ"],
        "the page's headings, in order"
    );
}

/// The plain page (the user, 2026-11-01): the hero with the install box, the form, the FAQ, and
/// nothing between them.
#[test]
fn the_page_is_three_parts_and_nothing_more() {
    let main = page_main();
    let ids: Vec<String> = tags(main, "section").into_iter().map(|t| t.get("id").cloned().unwrap_or_default()).collect();
    assert_eq!(ids, ["top", "contact", "faq"], "the page's parts, in order");
    for gone in ["vibe-coders", "id=\"plans\"", "use-cases", "id=\"licensing\"", "<sup", "<blockquote", "class=\"pricing", "pricing-hero"] {
        assert!(!main.contains(gone), "the page still renders {gone:?}, which the lean layout removed");
    }
}





#[test]
fn the_page_carries_no_theme_attribute() {
    assert!(!contact_page().contains("data-theme"), "the contact page carries a data-theme attribute");
}

#[test]
fn no_mailbox_but_the_team_address() {
    assert_eq!(content::TEAM_EMAIL, "team@datacenter.dev");
    let shown: Vec<String> = mailboxes(&decode(contact_page()));
    assert!(shown.iter().any(|m| m == content::TEAM_EMAIL), "the page never names {}", content::TEAM_EMAIL);
    assert!(shown.iter().all(|m| m == content::TEAM_EMAIL), "the page names other mailboxes: {shown:?}");
    let mailtos: Vec<String> = links(contact_page()).into_iter().filter_map(|(a, _)| a.get("href").filter(|h| h.starts_with("mailto:")).cloned()).collect();
    assert!(!mailtos.is_empty(), "the page offers no email link");
    for href in mailtos {
        assert_eq!(href, format!("mailto:{}", content::TEAM_EMAIL));
    }
    let llms = mailboxes(&repo_file("apps/logicaffeine_web/public/llms.txt"));
    assert!(!llms.is_empty() && llms.iter().all(|m| m == content::TEAM_EMAIL), "llms.txt names {llms:?}");
}

#[test]
fn the_legal_pages_name_only_the_team_address() {
    for (page, route) in [("apps/logicaffeine_web/privacy.html", "/privacy"), ("apps/logicaffeine_web/terms.html", "/terms")] {
        let written = mailboxes(&decode(&repo_file(page)));
        assert!(!written.is_empty(), "{page} names no address to reach the licensor");
        let others: Vec<&String> = written.iter().filter(|m| *m != content::TEAM_EMAIL).collect();
        assert!(others.is_empty(), "{page} names mailboxes other than {}: {others:?}", content::TEAM_EMAIL);
        let shown = mailboxes(&decode(&render_route(route)));
        assert!(shown.iter().any(|m| m == content::TEAM_EMAIL), "{route} never shows {}", content::TEAM_EMAIL);
        assert!(shown.iter().all(|m| m == content::TEAM_EMAIL), "{route} shows {shown:?}");
    }
}

#[test]
fn the_team_address_is_spelled_once_in_source() {
    let mut sources: Vec<std::path::PathBuf> = Vec::new();
    let dir = repo_root().join("apps/logicaffeine_web/src/ui/pages/contact");
    for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display())) {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "rs") {
            sources.push(path);
        }
    }
    let mut spelled = Vec::new();
    for path in sources {
        let text = std::fs::read_to_string(&path).unwrap();
        for mailbox in mailboxes(&text) {
            spelled.push(format!("{}: {mailbox}", path.file_name().unwrap().to_string_lossy()));
        }
    }
    assert_eq!(spelled, [format!("content.rs: {}", content::TEAM_EMAIL)], "the pricing sources spell mailboxes");
}

// ---------------------------------------------------------------------------------------------
// 6. The form
// ---------------------------------------------------------------------------------------------

fn contact() -> &'static str {
    section(contact_page(), "contact")
}

/// The one input in the form whose `name` is `name`.
fn input(name: &str) -> BTreeMap<String, String> {
    let found: Vec<_> = tags(contact(), "input").into_iter().filter(|i| i.get("name").is_some_and(|n| n == name)).collect();
    assert_eq!(found.len(), 1, "the form has {} inputs named {name:?}", found.len());
    found.into_iter().next().unwrap()
}

/// The text of the label bound to the input with `id`.
fn label_for(id: &str) -> String {
    let form = contact();
    let needle = format!("for=\"{id}\"");
    let at = form.find(&needle).unwrap_or_else(|| panic!("no label is bound to #{id}"));
    let start = form[..at].rfind("<label").expect("the for= sits on a label");
    let end = start + form[start..].find("</label>").expect("a closed label");
    visible_text(&form[start..end])
}

#[test]
fn the_contact_section_holds_one_form() {
    let form_tags = tags(contact(), "form");
    assert_eq!(form_tags.len(), 1, "the contact section holds {} forms", form_tags.len());
    assert!(is_set(&form_tags[0], "novalidate"), "the form leaves validation to the browser's bubbles; the page validates and says why in words");
    let heading = contact().split("<h2").nth(1).map(|t| visible_text(&t[t.find('>').unwrap() + 1..t.find("</h2>").unwrap()]));
    assert_eq!(heading.as_deref(), Some(content::CONTACT_HEADING));
}

#[test]
fn the_email_field_is_required_and_says_so() {
    let email = input("email");
    assert_eq!(email.get("type").map(String::as_str), Some("email"));
    assert_eq!(email.get("autocomplete").map(String::as_str), Some("email"));
    assert!(is_set(&email, "required"), "the email input is not required");
    let id = email.get("id").expect("the email input has an id");
    let label = label_for(id);
    assert!(label.contains("Work email") && label.contains("(required)"), "the email label reads {label:?}");
}

#[test]
fn the_phone_field_is_optional_and_says_so() {
    let phone = input("phone");
    assert_eq!(phone.get("type").map(String::as_str), Some("tel"));
    assert_eq!(phone.get("inputmode").map(String::as_str), Some("tel"));
    assert_eq!(phone.get("autocomplete").map(String::as_str), Some("tel"));
    assert!(!is_set(&phone, "required"), "the phone input is required (plan D8: it is optional)");
    assert!(!phone.contains_key("pattern"), "the phone input restricts its format; any format is accepted");
    let id = phone.get("id").expect("the phone input has an id");
    let label = label_for(id);
    assert!(label.contains("Phone") && label.contains("(optional)"), "the phone label reads {label:?}");
    let hint_id = phone.get("aria-describedby").expect("the phone input is described by its hint");
    let hint = contact().split(&format!("id=\"{}\"", hint_id.split_whitespace().next().unwrap())).nth(1).expect("the hint exists");
    let hint = visible_text(&hint[hint.find('>').unwrap() + 1..hint.find("</").unwrap()]);
    assert_eq!(hint, "If you'd like a call back. Never sold, never autodialed.");
}

#[test]
fn the_honeypot_is_hidden_from_everyone() {
    let trap = input("website");
    assert_eq!(trap.get("tabindex").map(String::as_str), Some("-1"), "the honeypot is reachable by Tab");
    assert_eq!(trap.get("autocomplete").map(String::as_str), Some("off"), "browsers may autofill the honeypot");
    let form = contact();
    let at = form.find("name=\"website\"").unwrap();
    let hidden_parent = form[..at].rfind("aria-hidden=\"true\"").expect("the honeypot sits in an aria-hidden container");
    let between = &form[hidden_parent..at];
    assert!(!between.contains("</div>"), "the aria-hidden container closes before the honeypot");
}

#[test]
fn the_form_speaks_its_state_and_links_its_privacy_note() {
    let form = contact();
    let status = tags(form, "div").into_iter().chain(tags(form, "p")).find(|t| t.get("role").is_some_and(|r| r == "status")).expect("a status region");
    assert_eq!(status.get("aria-live").map(String::as_str), Some("polite"));
    assert_eq!(status.get("tabindex").map(String::as_str), Some("-1"), "the status cannot take focus after a send");
    let submit: Vec<_> = tags(form, "button").into_iter().filter(|b| b.get("type").is_some_and(|t| t == "submit")).collect();
    assert_eq!(submit.len(), 1);
    assert!(form.contains(">Send<"), "the submit button does not read Send");
    let form_links = links(form);
    assert!(form_links.iter().any(|(a, _)| a.get("href").is_some_and(|h| h == "/privacy#pricing-requests")), "the form does not link its privacy note");
    let text = visible_text(form);
    assert!(text.contains("We use these only to reply to you."), "the form does not say what the details are for");
    assert!(text.contains(&format!("Prefer email? {}", content::TEAM_EMAIL)), "the form does not offer the team address");
}

#[test]
fn the_form_waits_for_the_app_before_it_sends() {
    // The prerendered form has no app behind it: a submit there would send the fields as a
    // query string to a page that drops them. Its button stays disabled until the app takes over.
    let form = contact();
    let submit = tags(form, "button").into_iter().find(|b| b.get("type").is_some_and(|t| t == "submit")).unwrap();
    assert!(is_set(&submit, "disabled"), "the prerendered submit button is live");
}

#[test]
fn the_privacy_policy_explains_pricing_requests() {
    let html = render_route("/privacy");
    let at = html.find("id=\"pricing-requests\"").expect("the privacy policy has a pricing-requests section");
    let end = at + html[at..].find("</div>").expect("the pricing-requests paragraph closes");
    let text = visible_text(&html[at..end]);
    for needle in ["pricing", "email", "phone", "reply", "Cloudflare", "24 months", "never sell", "marketing", "Turnstile", "Resend"] {
        assert!(text.to_lowercase().contains(&needle.to_lowercase()), "the pricing-requests paragraph does not mention {needle:?}: {text}");
    }
}

// ---------------------------------------------------------------------------------------------
// The form and the worker behind it (`services/leads`) speak one contract
// ---------------------------------------------------------------------------------------------

/// The string a JavaScript `const NAME = '…';` binds.
fn js_string_const(source: &str, name: &str) -> String {
    let prefix = format!("const {name} = ");
    let line = source.lines().map(str::trim).find(|l| l.starts_with(&prefix)).unwrap_or_else(|| panic!("no `const {name}`"));
    let value = line[prefix.len()..].trim_end_matches(';');
    let quote = value.chars().next().filter(|q| *q == '\'' || *q == '"').unwrap_or_else(|| panic!("`const {name}` is not a string: {value}"));
    assert!(value.len() >= 2 && value.ends_with(quote), "`const {name}` is not one plain string: {value}");
    value[1..value.len() - 1].to_string()
}

/// The pattern a JavaScript `const NAME = /…/;` binds.
fn js_regex_const(source: &str, name: &str) -> String {
    let prefix = format!("const {name} = /");
    let line = source.lines().map(str::trim).find(|l| l.starts_with(&prefix)).unwrap_or_else(|| panic!("no `const {name} = /…/`"));
    let rest = &line[prefix.len()..];
    rest[..rest.rfind('/').expect("a closed regex literal")].to_string()
}

/// Every value a TOML `key = "value"` line gives, in file order.
fn toml_strings(source: &str, key: &str) -> Vec<String> {
    source
        .lines()
        .map(str::trim)
        .filter_map(|l| l.strip_prefix(key).map(str::trim_start).and_then(|r| r.strip_prefix('=')))
        .map(|r| r.trim().trim_matches('"').to_string())
        .collect()
}

/// Whether `value` matches a rule of the one shape the worker's source rule has, `^[class]{min,max}$`,
/// whose class is single characters and `a-z` ranges (a `-` last is itself).
fn bracket_rule_matches(rule: &str, value: &str) -> bool {
    let body = rule.strip_prefix("^[").and_then(|r| r.strip_suffix('$')).unwrap_or_else(|| panic!("the rule is not ^[…]{{m,n}}$: {rule}"));
    let (class, counts) = body.split_once("]{").unwrap_or_else(|| panic!("the rule has no {{m,n}}: {rule}"));
    let (min, max) = counts.strip_suffix('}').and_then(|c| c.split_once(',')).expect("a {min,max} count");
    let (min, max): (usize, usize) = (min.parse().unwrap(), max.parse().unwrap());
    let class: Vec<char> = class.chars().collect();
    let mut allowed = Vec::new();
    let mut i = 0;
    while i < class.len() {
        if i + 2 < class.len() && class[i + 1] == '-' {
            allowed.extend(class[i]..=class[i + 2]);
            i += 3;
        } else {
            allowed.push(class[i]);
            i += 1;
        }
    }
    let n = value.chars().count();
    (min..=max).contains(&n) && value.chars().all(|c| allowed.contains(&c))
}

#[test]
fn the_rule_reader_reads_the_rule_it_is_given() {
    let rule = "^[A-Za-z0-9._/#-]{1,64}$";
    let (longest, too_long) = ("a".repeat(64), "a".repeat(65));
    for good in ["pricing", "contact", "/contact#contact", "a", "x-y_z.w", longest.as_str()] {
        assert!(bracket_rule_matches(rule, good), "{good:?}");
    }
    for bad in ["", "two words", "pricing?", "é", too_long.as_str()] {
        assert!(!bracket_rule_matches(rule, bad), "{bad:?}");
    }
    assert_eq!(js_string_const("  const A = 'x-y';\n", "A"), "x-y");
    assert_eq!(js_regex_const("const R = /^[a-z]{1,2}$/;\n", "R"), "^[a-z]{1,2}$");
    assert_eq!(toml_strings("[vars]\nNOTIFY_TO = \"a@b.co\"\nx = 1\n", "NOTIFY_TO"), ["a@b.co"]);
}

/// Four constants live twice, once in Rust and once in the worker's JavaScript and config, and a
/// mismatch in any one fails every request at run time and nowhere sooner.
#[test]
fn the_form_and_the_worker_agree_on_the_contract() {
    let index = repo_file("services/leads/src/index.js");
    let rules = repo_file("services/leads/src/lead.js");
    let wrangler = repo_file("services/leads/wrangler.toml");

    let action = js_string_const(&index, "TURNSTILE_ACTION");
    assert_eq!(lead::TURNSTILE_ACTION, action, "the form's Turnstile widget stamps an action the worker's siteverify check refuses");

    let routes = toml_strings(&wrangler, "pattern");
    assert_eq!(routes.len(), 1, "the worker has one route: {routes:?}");
    assert_eq!(lead::ENDPOINT, format!("https://{}", routes[0]), "the form posts to an address the worker's route does not serve");
    let path = js_string_const(&index, "LEAD_PATH");
    assert!(lead::ENDPOINT.ends_with(&path), "the form posts to {} but the worker answers only {path}", lead::ENDPOINT);

    let source_rule = js_regex_const(&rules, "SOURCE");
    assert!(bracket_rule_matches(&source_rule, lead::SOURCE), "the worker refuses the form's source {:?} ({source_rule})", lead::SOURCE);

    assert_eq!(toml_strings(&wrangler, "NOTIFY_TO"), [content::TEAM_EMAIL], "the worker mails an address the page does not offer");
}

/// LOGOS is MIT. No page the site renders, no doc, and nothing the agent pack mirrors says a license
/// gates anything, names the old Business Source terms, or asks for a key.
#[test]
fn no_shipped_surface_speaks_of_a_license_gate() {
    const BANNED: &[&str] = &[
        "license-gated", "license gated", "Business Source", "BUSL", "commercial license", "Logic Service",
        "license key", "LOGOS_LICENSE", "--license", "Pro+",
    ];
    let mut surfaces: Vec<(String, String)> =
        every_page().iter().map(|(path, html)| (path.clone(), visible_text(html))).collect();
    for rel in [
        "README.md",
        "docs/project/FAQ.md",
        "apps/logicaffeine_web/public/llms.txt",
        "apps/logicaffeine_web/public/llms-full.txt",
        "apps/logicaffeine_web/public/agent.txt",
    ] {
        surfaces.push((rel.to_string(), repo_file(rel)));
    }
    for dir in ["docs", "apps/logicaffeine_cli/assets/agent/docs"] {
        let mut entries: Vec<_> = std::fs::read_dir(repo_root().join(dir))
            .expect(dir)
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "md"))
            .collect();
        entries.sort();
        for path in entries {
            let text = std::fs::read_to_string(&path).unwrap();
            surfaces.push((path.strip_prefix(repo_root()).unwrap().display().to_string(), text));
        }
    }
    assert!(surfaces.len() > 30, "the scan covers {} surfaces", surfaces.len());
    let found: Vec<String> = surfaces
        .iter()
        .flat_map(|(name, text)| BANNED.iter().filter(|b| text.contains(*b)).map(move |b| format!("{name} says {b:?}")))
        .collect();
    assert!(found.is_empty(), "surfaces that still speak of a license gate:\n{}", found.join("\n"));
}
