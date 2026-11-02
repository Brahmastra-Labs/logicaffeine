//! The share card: the image a link to any page of the site unfurls with. The user (2026-10-01):
//! "yes we can revamp the OG photo as well to make it clean and slick when sending over imessage".
//!
//! The card is a page, `apps/logicaffeine_web/share-card/card.html`, that
//! `scripts/web/render-share-card.sh` draws into `public/assets/OG-photo.png`. The drawing records
//! the digest of every file it was drawn from, so an edit to the card that is never redrawn is
//! caught here, and the card's words are the front page's own, so a new heading reaches the card.

#![cfg(feature = "web-tests")]

#[path = "web_common/page_html.rs"]
mod page_html;

use logicaffeine_web::ui::components::icon::IconVariant;
use logicaffeine_web::ui::pages::landing::content::{COPY_LABEL, HEADING};
use page_html::*;
use std::path::{Path, PathBuf};

const CARD: &str = "apps/logicaffeine_web/share-card/card.html";
const IMAGE: &str = "apps/logicaffeine_web/public/assets/OG-photo.png";
const RENDERER: &str = "scripts/web/render-share-card.mjs";
const KEYWORD: &str = "logos-share-card";

/// Large-image cards are drawn 1.91:1, and this is the size every page's head declares.
const WIDTH: u32 = 1200;
const HEIGHT: u32 = 630;

/// A link preview fetches the card on the sender's phone, often on a cellular connection.
const BYTE_BUDGET: usize = 256 * 1024;

/// The user (2026-10-01): "we can icnlude "Give your agent powerful logic, math, formal verification,
/// and programming tools, all in one, lightweight package." All agents love it."
const AGENT_LINE: &str =
    "Give your agent powerful logic, math, formal verification, and programming tools, all in one, lightweight package.";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &str) -> Vec<u8> {
    std::fs::read(root().join(path)).unwrap_or_else(|e| panic!("cannot read {path}: {e}"))
}

fn card_html() -> String {
    String::from_utf8(read(CARD)).expect("the card is UTF-8")
}

struct Chunk {
    kind: String,
    data: Vec<u8>,
    crc: u32,
}

fn chunks(png: &[u8]) -> Vec<Chunk> {
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "{IMAGE} is not a PNG");
    let mut out = Vec::new();
    let mut at = 8;
    while at < png.len() {
        assert!(at + 12 <= png.len(), "{IMAGE} ends inside a chunk header at byte {at}");
        let length = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
        let end = at + 8 + length;
        assert!(end + 4 <= png.len(), "{IMAGE} ends inside a chunk at byte {at}");
        out.push(Chunk {
            kind: String::from_utf8_lossy(&png[at + 4..at + 8]).into_owned(),
            data: png[at + 8..end].to_vec(),
            crc: u32::from_be_bytes(png[end..end + 4].try_into().unwrap()),
        });
        at = end + 4;
    }
    out
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, &byte| (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3))
}

/// The value of every `src="…"` and `url("…")` in the card, in order.
fn references(html: &str) -> Vec<String> {
    let mut found = Vec::new();
    for opener in ["src=\"", "url(\""] {
        let mut rest = html;
        while let Some(at) = rest.find(opener) {
            let value = &rest[at + opener.len()..];
            let end = value.find('"').expect("a closed reference");
            found.push((html.len() - rest.len() + at, value[..end].to_string()));
            rest = &value[end..];
        }
    }
    found.sort();
    found.into_iter().map(|(_, value)| value).collect()
}

fn is_remote(reference: &str) -> bool {
    reference.split_once(':').is_some_and(|(scheme, _)| scheme.chars().all(|c| c.is_ascii_lowercase()))
}

/// A local reference, as a path from the repository root.
fn from_root(reference: &str) -> String {
    let mut parts: Vec<&str> = Path::new(CARD).parent().unwrap().to_str().unwrap().split('/').collect();
    for part in reference.split('/') {
        match part {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            part => parts.push(part),
        }
    }
    parts.join("/")
}

/// The files the card is drawn from: the card, then each local file it references, once each.
fn drawn_from() -> Vec<String> {
    let mut files = vec![CARD.to_string()];
    for reference in references(&card_html()) {
        if !is_remote(&reference) && !files.contains(&from_root(&reference)) {
            files.push(from_root(&reference));
        }
    }
    files
}

/// What the image says it was drawn from: `(digest, path)` per line of its `logos-share-card` text.
fn recorded() -> Vec<(String, String)> {
    let png = read(IMAGE);
    let texts: Vec<String> = chunks(&png)
        .into_iter()
        .filter(|chunk| chunk.kind == "tEXt")
        .filter_map(|chunk| {
            let (keyword, text) = chunk.data.split_at(chunk.data.iter().position(|&b| b == 0)?);
            (keyword == KEYWORD.as_bytes()).then(|| text[1..].iter().map(|&b| b as char).collect())
        })
        .collect();
    assert_eq!(
        texts.len(),
        1,
        "{IMAGE} does not say what it was drawn from (one tEXt chunk `{KEYWORD}`): it was not drawn by \
         scripts/web/render-share-card.sh"
    );
    texts[0]
        .lines()
        .map(|line| {
            let (digest, path) = line.split_once(' ').unwrap_or_else(|| panic!("a recorded line without a path: {line:?}"));
            (digest.to_string(), path.to_string())
        })
        .collect()
}

#[test]
fn the_digest_is_fnv1a_64() {
    assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
    assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
}

#[test]
fn the_checksum_is_the_pngs_crc32() {
    assert_eq!(crc32(b"IEND"), 0xae42_6082, "the CRC every PNG's last chunk carries");
    assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
}

#[test]
fn the_card_is_a_whole_png_of_the_size_the_head_declares() {
    let png = read(IMAGE);
    let chunks = chunks(&png);
    for chunk in &chunks {
        let mut checked = chunk.kind.as_bytes().to_vec();
        checked.extend_from_slice(&chunk.data);
        assert_eq!(crc32(&checked), chunk.crc, "the {} chunk of {IMAGE} is damaged", chunk.kind);
    }
    assert_eq!(chunks.first().map(|c| c.kind.as_str()), Some("IHDR"), "the header comes first");
    assert_eq!(chunks.last().map(|c| c.kind.as_str()), Some("IEND"), "the end comes last");
    let header = &chunks[0].data;
    let width = u32::from_be_bytes(header[0..4].try_into().unwrap());
    let height = u32::from_be_bytes(header[4..8].try_into().unwrap());
    assert_eq!((width, height), (WIDTH, HEIGHT), "the card is {width}x{height}");
}

#[test]
fn the_card_fits_its_byte_budget() {
    let bytes = read(IMAGE).len();
    assert!(bytes <= BYTE_BUDGET, "the card is {bytes} bytes, over its budget of {BYTE_BUDGET}");
}

#[test]
fn the_card_was_drawn_from_its_source_as_it_stands() {
    let recorded = recorded();
    let paths: Vec<&str> = recorded.iter().map(|(_, path)| path.as_str()).collect();
    assert_eq!(paths, drawn_from(), "the image was drawn from other files than the card names now");
    for (digest, path) in &recorded {
        let now = format!("{:016x}", fnv1a64(&read(path)));
        assert_eq!(
            digest, &now,
            "{path} changed after the card was drawn: run scripts/web/render-share-card.sh"
        );
    }
}

#[test]
fn the_card_says_the_front_page_heading() {
    let html = card_html();
    let headings = elements(&html, "h1");
    assert_eq!(headings.len(), 1, "the card has one heading");
    let lines = element_texts(headings[0], "span");
    assert_eq!(lines, HEADING, "the card's heading, line by line, is the front page's");
    assert_eq!(visible_text(headings[0]), HEADING.join(" "), "the lines read as three sentences");
}

#[test]
fn the_card_says_what_an_agent_gets_under_its_heading() {
    let html = card_html();
    let lines: Vec<&str> = elements(&html, "p").into_iter().filter(|p| has_class(&own_attributes(p), "lede")).collect();
    assert_eq!(lines.len(), 1, "the card has one line under its heading");
    assert_eq!(visible_text(lines[0]), AGENT_LINE, "the line is the user's, word for word");
    let at = |needle: &str| html.find(needle).unwrap_or_else(|| panic!("the card has no {needle}"));
    assert!(
        at("<h1") < at("class=\"lede\"") && at("class=\"lede\"") < at("class=\"copy\""),
        "the heading, then the line, then the button"
    );
}

#[test]
fn the_cards_button_is_the_front_pages_copy_button() {
    let html = card_html();
    let button = elements(&html, "p")
        .into_iter()
        .find(|p| has_class(&own_attributes(p), "copy"))
        .expect("the card shows the copy button");
    assert_eq!(visible_text(button), COPY_LABEL, "the button's words are the front page's");
    assert!(button.contains(IconVariant::Copy.svg_content()), "the button shows the front page's copy symbol");
}

#[test]
fn the_card_shows_the_sites_logo() {
    let html = card_html();
    let logos: Vec<String> = tags(&html, "img").into_iter().filter_map(|img| img.get("src").cloned()).collect();
    assert_eq!(logos, ["../assets/logo.svg"], "the card's one image is the logo the nav draws");
    assert_eq!(from_root(&logos[0]), "apps/logicaffeine_web/assets/logo.svg");
}

#[test]
fn the_cards_words_are_plain() {
    let html = card_html();
    let body = elements(&html, "body");
    assert_eq!(body.len(), 1, "the card has one body");
    let text = visible_text(body[0]);
    for (mark, name) in [('\u{2014}', "an em dash"), ('\u{2013}', "an en dash"), (';', "a semicolon")] {
        assert!(!text.contains(mark), "the card's words carry {name}: {text:?}");
    }
}

#[test]
fn the_card_draws_only_from_files_that_stay_put() {
    for reference in references(&card_html()) {
        if is_remote(&reference) {
            let pinned = reference
                .strip_prefix("https://cdn.jsdelivr.net/npm/")
                .and_then(|rest| rest.trim_start_matches('@').split_once('@'))
                .and_then(|(_, version)| version.split_once('/'))
                .is_some_and(|(version, _)| {
                    let parts: Vec<&str> = version.split('.').collect();
                    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
                });
            assert!(pinned, "the card draws from {reference}, which is not one published version");
        } else {
            let file = from_root(&reference);
            assert!(root().join(&file).is_file(), "the card references {reference}, and {file} does not exist");
        }
    }
}

#[test]
fn the_renderer_draws_this_card_into_this_image() {
    let script = String::from_utf8(read(RENDERER)).expect("the renderer is UTF-8");
    for line in [
        format!("const CARD = '{CARD}';"),
        format!("const OUT = '{IMAGE}';"),
        format!("const WIDTH = {WIDTH};"),
        format!("const HEIGHT = {HEIGHT};"),
        format!("const KEYWORD = '{KEYWORD}';"),
    ] {
        assert!(script.contains(&line), "{RENDERER} lacks `{line}`");
    }
}
