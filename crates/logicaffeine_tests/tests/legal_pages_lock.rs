//! The public legal pages carry a business contact, never a personal one, both builds publish the same bytes, and
//! the terms leave LOGOS to its MIT License.
//!
//! `apps/logicaffeine_web/{terms,privacy}.html` are compiled into the native/SSG build
//! (`include_str!`) and staged by `scripts/web/stage-web-data.sh` into the gitignored
//! `apps/logicaffeine_web/public/data/` for the wasm build to fetch. The lock runs that staging step
//! into a scratch directory, so it holds on any checkout, staged or not. The pages name Datacenter.Dev
//! and reach it by email. A phone number or a street address in them is a person's, and it is
//! published the moment the site deploys. So every phone number and street address in the rendered
//! text must be on the allowlist below, where each entry is a public institution the page is legally
//! required to cite.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

const PAGES: &[&str] = &["terms.html", "privacy.html"];

/// Contact details of public institutions the pages must cite, each with why.
const INSTITUTIONAL: &[(&str, &str)] = &[
    (
        "(800) 952-5210",
        "California Department of Consumer Affairs, required by Cal. Civ. Code § 1789.3",
    ),
    (
        "(916) 445-1254",
        "California Department of Consumer Affairs, required by Cal. Civ. Code § 1789.3",
    ),
    (
        "1625 North Market Blvd.",
        "California Department of Consumer Affairs, required by Cal. Civ. Code § 1789.3",
    ),
];

fn read(rel: &str) -> String {
    let p = repo_root().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

/// The text a reader sees: tags, scripts and styles removed, `&nbsp;` read as a space, and every
/// whitespace run collapsed to one space, as a browser renders it.
fn visible_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let mut rest = html;
    while let Some(lt) = rest.find('<') {
        out.push_str(&rest[..lt]);
        out.push(' ');
        let tag = &rest[lt..];
        let lower = tag.get(..8).unwrap_or(tag).to_ascii_lowercase();
        let close = if lower.starts_with("<script") {
            tag.to_ascii_lowercase()
                .find("</script>")
                .map(|i| i + "</script>".len())
        } else if lower.starts_with("<style") {
            tag.to_ascii_lowercase()
                .find("</style>")
                .map(|i| i + "</style>".len())
        } else {
            tag.find('>').map(|i| i + 1)
        };
        rest = &tag[close.unwrap_or(tag.len())..];
    }
    out.push_str(rest);
    out.replace("&nbsp;", " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Phone-number-shaped runs: `(800) 952-5210`, `800-952-5210`, `800.952.5210`, `8009525210`.
fn phone_numbers(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() || chars[i] == '(' {
            let start = i;
            let mut digits = 0;
            let mut j = i;
            while j < chars.len()
                && (chars[j].is_ascii_digit() || "()-. ".contains(chars[j]))
                && digits < 11
            {
                if chars[j].is_ascii_digit() {
                    digits += 1;
                }
                j += 1;
            }
            let run: String = chars[start..j].iter().collect::<String>();
            let run = run
                .trim_end_matches(|c: char| !c.is_ascii_digit())
                .trim()
                .to_string();
            let only_digits: String = run.chars().filter(|c| c.is_ascii_digit()).collect();
            let word_boundary_before = start == 0 || !chars[start - 1].is_ascii_alphanumeric();
            if only_digits.len() == 10 && word_boundary_before {
                found.push(run);
                i = j;
                continue;
            }
        }
        i += 1;
    }
    found
}

/// Street-address-shaped runs: a house number, one to three capitalized words, a street type.
fn street_addresses(text: &str) -> Vec<String> {
    const TYPES: &[&str] = &[
        "Street",
        "St.",
        "Avenue",
        "Ave",
        "Ave.",
        "Road",
        "Rd",
        "Rd.",
        "Boulevard",
        "Blvd",
        "Blvd.",
        "Drive",
        "Dr.",
        "Lane",
        "Ln",
        "Ln.",
        "Court",
        "Ct",
        "Ct.",
        "Way",
        "Place",
        "Pl.",
    ];
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut found = Vec::new();
    for (i, w) in words.iter().enumerate() {
        if !(1..=6).contains(&w.len()) || !w.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        for n in 1..=3 {
            let Some(name) = words.get(i + 1..i + 1 + n) else {
                break;
            };
            if !name
                .iter()
                .all(|x| x.chars().next().is_some_and(|c| c.is_ascii_uppercase()))
            {
                break;
            }
            let Some(kind) = words.get(i + 1 + n) else {
                break;
            };
            let kind = kind.trim_end_matches(',');
            if TYPES.contains(&kind) {
                found.push(format!("{w} {} {kind}", name.join(" ")));
            }
        }
    }
    found
}

#[test]
fn each_legal_page_is_staged_verbatim() {
    let scratch = tempfile::tempdir().expect("scratch dir");
    let out = scratch.path().join("data");
    let run = Command::new("bash")
        .arg(repo_root().join("scripts/web/stage-web-data.sh"))
        .arg(&out)
        .output()
        .expect("bash");
    assert!(
        run.status.success(),
        "scripts/web/stage-web-data.sh failed to stage the web data bundle:\n{}\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    for page in PAGES {
        let root_copy = std::fs::read(repo_root().join(format!("apps/logicaffeine_web/{page}")))
            .expect("crate-root page");
        let staged = std::fs::read(out.join(page))
            .unwrap_or_else(|e| panic!("scripts/web/stage-web-data.sh did not stage {page}: {e}"));
        assert!(
            root_copy == staged,
            "scripts/web/stage-web-data.sh staged {page} with bytes that differ from apps/logicaffeine_web/{page} — \
             the native build and the wasm build would publish different legal terms. The staging step copies \
             the crate-root page verbatim."
        );
    }
}

#[test]
fn no_legal_page_publishes_a_personal_phone_number_or_street_address() {
    let mut problems = Vec::new();
    for page in PAGES {
        let text = visible_text(&read(&format!("apps/logicaffeine_web/{page}")));
        let allowed = |s: &str| {
            INSTITUTIONAL
                .iter()
                .any(|(entry, _)| s.contains(entry) || entry.contains(s))
        };
        for phone in phone_numbers(&text) {
            if !allowed(&phone) {
                problems.push(format!("{page}: phone number `{phone}`"));
            }
        }
        for addr in street_addresses(&text) {
            if !allowed(&addr) {
                problems.push(format!("{page}: street address `{addr}`"));
            }
        }
    }
    assert!(
        problems.is_empty(),
        "the legal pages publish contact details that are not a public institution's — the business is \
         reached by email; a personal phone or street address must not ship:\n  {}",
        problems.join("\n  ")
    );
}

/// The detectors must see the shapes they claim to, or the lock above passes on nothing.
#[test]
fn the_detectors_see_what_they_claim_to() {
    let sample = visible_text(
        "<p>Call <b>2025550143</b> or (800) 952-5210 or 916.445.1254.</p><style>x{}</style>\
         <p>Mail us at 742 Maple Street, Unit B or 1625 North Market Blvd., Suite N</p>\
         <p>Version 2026 Update and 12 Days Of Code</p>",
    );
    assert_eq!(
        phone_numbers(&sample),
        vec!["2025550143", "(800) 952-5210", "916.445.1254"]
    );
    assert_eq!(
        street_addresses(&sample),
        vec!["742 Maple Street", "1625 North Market Blvd."]
    );
    for (entry, why) in INSTITUTIONAL {
        let text = visible_text(&read("apps/logicaffeine_web/terms.html"));
        assert!(text.contains(entry), "allowlisted `{entry}` ({why}) no longer appears in terms.html — drop it from the allowlist");
    }
}

/// LOGOS is MIT, and the terms say so where they speak of software: §2 puts the software, the Services' own code
/// included, under the MIT License and lets that license win where the two differ, §7 steps aside for it, and each
/// prohibited activity that would forbid what the license grants (copying and adapting the code, taking it apart,
/// earning from it) excepts LOGOS. The page is dated by its last change.
#[test]
fn the_terms_leave_the_software_to_the_mit_license() {
    let text = visible_text(&read("apps/logicaffeine_web/terms.html"));
    for clause in [
        "Last updated November 2, 2026",
        "Open source software LOGOS, the software these Services run and offer, including the source code of the Services themselves, is open source under the MIT License",
        "nothing in these Legal Terms limits the rights it grants.",
        "Where these Legal Terms and the MIT License differ about that software, the MIT License controls.",
        "LOGOS is licensed to you under the MIT License, as the \"Open source software\" part of Section 2 sets out, and this section does not apply to it.",
        "or other code, except as the MIT License permits for LOGOS.",
        "Except as permitted by applicable law or by the MIT License for LOGOS, decipher, decompile, disassemble, or reverse engineer",
        "for any revenue-generating endeavor or commercial enterprise, except LOGOS itself, which the MIT License lets anyone use for any purpose.",
    ] {
        assert!(text.contains(clause), "terms.html does not say: {clause:?}");
    }
}
