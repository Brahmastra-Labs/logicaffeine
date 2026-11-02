//! LOCK — `on-<event>.<qualifier>` is read through ONE catalogue, and a word it does not know is a
//! compile error that names the fix.
//!
//! `lgx::html::EventModifier` classifies every qualifier: the behavioural ones (`.prevent`,
//! `.stop`, `.once`, `.capture`, `.passive`), a closed set of named keys plus any single printable
//! character, and the system keys. The template parse classifies once; the view IR carries the
//! classification; every tier reads it. This file holds the catalogue to its tables, the parse to
//! its refusals, the tiers to having no rule of their own, and every taught example to words the
//! catalogue knows.

#[path = "language_common/mod.rs"]
mod common;

use common::refusal;
use logicaffeine_language::lgx::html::{
    classify_modifiers, Behaviour, EventModifier, KeyName, ModifierError, SystemKey,
    BEHAVIOUR_MODIFIERS, KEY_EVENTS, KEY_MODIFIERS, SYSTEM_KEY_MODIFIERS,
};
use std::path::{Path, PathBuf};

/// A page whose one input carries the handler `attr`.
fn page(attr: &str) -> String {
    format!(
        "## Component App\nState:\n    Let n be 0.\nActions:\n    To add:\n        Set n to n + 1.\n\
         View:\n    <p id=\"p\">{{n}}</p>\n    <input id=\"d\" aria-label=\"d\" {attr}={{add}}>\n\n\
         ## Main\nMount App.\n"
    )
}

// ── the tables ──────────────────────────────────────────────────────────────────────────────────

#[test]
fn every_catalogued_word_classifies_to_its_own_row() {
    for (word, b) in BEHAVIOUR_MODIFIERS {
        assert_eq!(
            EventModifier::classify(word),
            Some(EventModifier::Behaviour(*b)),
            "{word}"
        );
        assert_eq!(
            EventModifier::Behaviour(*b).word(),
            *word,
            "{word} prints back as itself"
        );
    }
    for (word, key) in KEY_MODIFIERS {
        assert_eq!(
            EventModifier::classify(word),
            Some(EventModifier::Key(KeyName::Named(key))),
            "{word}"
        );
        assert_eq!(
            EventModifier::Key(KeyName::Named(key)).word(),
            *word,
            "{word} prints back as itself"
        );
    }
    for (word, s) in SYSTEM_KEY_MODIFIERS {
        assert_eq!(
            EventModifier::classify(word),
            Some(EventModifier::System(*s)),
            "{word}"
        );
        assert_eq!(
            EventModifier::System(*s).word(),
            *word,
            "{word} prints back as itself"
        );
    }
}

#[test]
fn the_tables_cover_exactly_the_taught_vocabulary() {
    let behaviours: Vec<&str> = BEHAVIOUR_MODIFIERS.iter().map(|(w, _)| *w).collect();
    assert_eq!(
        behaviours,
        ["prevent", "stop", "once", "capture", "passive"]
    );
    let keys: Vec<&str> = KEY_MODIFIERS.iter().map(|(w, _)| *w).collect();
    assert_eq!(
        keys,
        [
            "enter",
            "escape",
            "space",
            "tab",
            "backspace",
            "delete",
            "arrowup",
            "arrowdown",
            "arrowleft",
            "arrowright",
            "home",
            "end",
            "pageup",
            "pagedown"
        ]
    );
    let systems: Vec<&str> = SYSTEM_KEY_MODIFIERS.iter().map(|(w, _)| *w).collect();
    assert_eq!(systems, ["ctrl", "alt", "shift", "meta"]);
    assert_eq!(KEY_EVENTS, ["keydown", "keyup", "keypress"]);
}

#[test]
fn a_named_key_carries_the_spelling_a_browser_reports() {
    // `KeyboardEvent.key` is what `eventKey` returns and what a driving `Press` names, so the
    // second column is the value a handler compares against.
    let reported = |word: &str| match EventModifier::classify(word) {
        Some(EventModifier::Key(k)) => k.event_key(),
        other => panic!("{word} must classify as a key, got {other:?}"),
    };
    assert_eq!(reported("enter"), "Enter");
    assert_eq!(reported("escape"), "Escape");
    assert_eq!(
        reported("space"),
        " ",
        "a browser reports the space bar as a space"
    );
    assert_eq!(reported("arrowup"), "ArrowUp");
    assert_eq!(reported("pagedown"), "PageDown");
    assert_eq!(reported("a"), "a");
}

#[test]
fn no_word_sits_in_two_tables() {
    let mut seen: Vec<&str> = Vec::new();
    for w in EventModifier::every_word() {
        assert!(!seen.contains(&w), "`{w}` is catalogued twice");
        assert!(
            w.chars().count() > 1,
            "`{w}`: a one-character qualifier is the key of that character"
        );
        seen.push(w);
    }
}

#[test]
fn a_single_printable_character_is_its_own_key() {
    for c in ['a', 'Z', '1', '?', '/', 'é'] {
        assert_eq!(
            EventModifier::classify(&c.to_string()),
            Some(EventModifier::Key(KeyName::Char(c))),
            "`.{c}`"
        );
    }
    assert_eq!(
        EventModifier::classify(" "),
        None,
        "whitespace is not a key qualifier"
    );
    assert_eq!(EventModifier::classify("\t"), None);
    assert_eq!(EventModifier::classify(""), None);
    assert_eq!(
        EventModifier::classify("ab"),
        None,
        "two characters are a word, and not a catalogued one"
    );
}

#[test]
fn named_qualifiers_match_case_insensitively() {
    assert_eq!(
        EventModifier::classify("Enter"),
        EventModifier::classify("enter")
    );
    assert_eq!(
        EventModifier::classify("PREVENT"),
        Some(EventModifier::Behaviour(Behaviour::Prevent))
    );
    assert_eq!(
        EventModifier::classify("Ctrl"),
        Some(EventModifier::System(SystemKey::Ctrl))
    );
}

// ── classification of one directive ────────────────────────────────────────────────────────────

#[test]
fn a_directive_classifies_in_order_and_drops_a_repeat() {
    let got = classify_modifiers("keydown", &["enter", "prevent", "prevent"]).unwrap();
    assert_eq!(
        got,
        vec![
            EventModifier::Key(KeyName::Named("Enter")),
            EventModifier::Behaviour(Behaviour::Prevent)
        ]
    );
    assert_eq!(EventModifier::key_of(&got), Some("Enter".to_string()));
    let prevent = Behaviour::Prevent.listen_bit();
    assert_eq!(
        EventModifier::host_listen_bits(&got) & prevent,
        prevent,
        "`.prevent` reaches the host"
    );
    assert_eq!(
        EventModifier::key_of(&[]),
        None,
        "no key qualifier answers every key"
    );
    assert_eq!(
        EventModifier::host_listen_bits(&[EventModifier::Behaviour(Behaviour::Stop)]) & prevent,
        0,
        "`.stop` alone cancels no default action"
    );
}

#[test]
fn the_same_key_twice_is_one_key() {
    assert!(classify_modifiers("keyup", &["enter", "Enter"]).is_ok());
}

#[test]
fn directive_refusals_name_their_qualifier_and_position() {
    assert_eq!(
        classify_modifiers("submit", &["prevnt"]),
        Err((
            0,
            ModifierError::Unknown {
                word: "prevnt".into(),
                suggestion: Some("prevent")
            }
        ))
    );
    assert_eq!(
        classify_modifiers("click", &["prevent", "enter"]),
        Err((
            1,
            ModifierError::KeyOnNonKeyEvent {
                word: "enter".into(),
                event: "click".into()
            }
        ))
    );
    assert_eq!(
        classify_modifiers("keydown", &["enter", "escape"]),
        Err((
            1,
            ModifierError::TwoKeys {
                first: "enter".into(),
                second: "escape".into()
            }
        ))
    );
    assert_eq!(
        classify_modifiers("keydown", &["ctrl", "enter"]),
        Err((
            0,
            ModifierError::SystemKeyNotDelivered {
                word: "ctrl".into()
            }
        ))
    );
    assert_eq!(
        classify_modifiers("click", &["zzzzzzzz"]),
        Err((
            0,
            ModifierError::Unknown {
                word: "zzzzzzzz".into(),
                suggestion: None
            }
        ))
    );
}

#[test]
fn prevent_and_passive_on_one_handler_are_refused_whichever_comes_first() {
    for pair in [["prevent", "passive"], ["passive", "prevent"]] {
        assert_eq!(
            classify_modifiers("wheel", &pair),
            Err((
                1,
                ModifierError::PreventOnPassive {
                    event: "wheel".into()
                }
            )),
            "{pair:?}: the second of the two contradicting qualifiers is the one refused"
        );
    }
    assert!(
        classify_modifiers("wheel", &["passive"]).is_ok(),
        "`.passive` alone is a promise, not a mistake"
    );
}

/// The bits a host's listen registration reads (`semantics::dom::LISTEN_*`), written here per
/// behaviour, so a catalogue row whose bit drifts from the host's — or a new row with no bit — is a
/// failure here rather than a flag a browser misreads.
#[test]
fn every_behaviour_carries_the_hosts_listen_bit() {
    use logicaffeine_compile::semantics::dom;
    let host = |b: Behaviour| match b {
        Behaviour::Prevent => dom::LISTEN_PREVENT,
        Behaviour::Stop => dom::LISTEN_STOP,
        Behaviour::Once => dom::LISTEN_ONCE,
        Behaviour::Capture => dom::LISTEN_CAPTURE,
        Behaviour::Passive => dom::LISTEN_PASSIVE,
    };
    let mut seen = 0u8;
    for (word, b) in BEHAVIOUR_MODIFIERS {
        assert_eq!(b.listen_bit(), host(*b), "`.{word}` carries the host's bit");
        assert_eq!(b.listen_bit().count_ones(), 1, "`.{word}` is one bit");
        assert_eq!(
            seen & b.listen_bit(),
            0,
            "`.{word}` shares no bit with another behaviour"
        );
        seen |= b.listen_bit();
    }
}

/// A host dispatches by every behaviour's bit but `.once`'s: the lowering keeps `.once` itself, and
/// a host never retires a listener on its own.
#[test]
fn a_hosts_listen_bits_are_every_behaviour_but_once() {
    for (word, b) in BEHAVIOUR_MODIFIERS {
        let bits = EventModifier::host_listen_bits(&[EventModifier::Behaviour(*b)]);
        let want = if matches!(b, Behaviour::Once) {
            0
        } else {
            b.listen_bit()
        };
        assert_eq!(bits, want, "`.{word}`");
    }
    let every: Vec<EventModifier> = BEHAVIOUR_MODIFIERS
        .iter()
        .map(|(_, b)| EventModifier::Behaviour(*b))
        .collect();
    let all_but_once = BEHAVIOUR_MODIFIERS
        .iter()
        .filter(|(_, b)| !matches!(b, Behaviour::Once))
        .fold(0u8, |bits, (_, b)| bits | b.listen_bit());
    assert_eq!(EventModifier::host_listen_bits(&every), all_but_once);
    let keyed = [EventModifier::Key(KeyName::Named("Enter"))];
    assert_eq!(
        EventModifier::host_listen_bits(&keyed),
        0,
        "a key carries no listen bit"
    );
}

// ── the parse refuses, with the caret on the qualifier ─────────────────────────────────────────

#[test]
fn a_passive_listener_that_prevents_is_a_compile_error_naming_both_fixes() {
    let (message, covered) = refusal(&page("on-keydown.prevent.passive"));
    assert_eq!(
        covered, ".passive",
        "the caret covers the contradicting qualifier: {message}"
    );
    assert!(
        message.contains("`.prevent` and `.passive` contradict each other"),
        "{message}"
    );
    assert!(
        message.contains("Did you mean `on-keydown.prevent={…}`"),
        "{message}"
    );
    assert!(message.contains("`on-keydown.passive={…}`"), "{message}");
}

#[test]
fn a_misspelt_qualifier_is_a_compile_error_with_did_you_mean() {
    let (message, covered) = refusal(&page("on-submit.prevnt"));
    assert_eq!(
        covered, ".prevnt",
        "the caret covers the qualifier: {message}"
    );
    assert!(
        message.contains("`.prevnt` is not an event qualifier"),
        "{message}"
    );
    assert!(message.contains("did you mean `.prevent`?"), "{message}");
}

#[test]
fn a_misspelt_key_suggests_the_key() {
    let (message, covered) = refusal(&page("on-keydown.entr"));
    assert_eq!(covered, ".entr");
    assert!(message.contains("did you mean `.enter`?"), "{message}");
    let (message, _) = refusal(&page("on-keydown.escpae"));
    assert!(message.contains("did you mean `.escape`?"), "{message}");
}

#[test]
fn the_caret_lands_on_the_refused_qualifier_among_several() {
    let (message, covered) = refusal(&page("on-keydown.enter.prevnt"));
    assert_eq!(covered, ".prevnt", "{message}");
}

#[test]
fn a_key_qualifier_on_a_click_is_refused() {
    let (message, covered) = refusal(&page("on-click.enter"));
    assert_eq!(covered, ".enter");
    assert!(message.contains("`click` is not a key event"), "{message}");
    assert!(
        message.contains("on-keydown.enter"),
        "the message shows the form that works: {message}"
    );
    let (_, covered) = refusal(&page("on-submit.a"));
    assert_eq!(covered, ".a", "a character key is a key too");
}

#[test]
fn two_keys_on_one_handler_are_refused() {
    let (message, covered) = refusal(&page("on-keydown.enter.escape"));
    assert_eq!(covered, ".escape");
    assert!(message.contains("a handler answers one key"), "{message}");
}

#[test]
fn every_system_key_is_refused_by_name() {
    for (word, _) in SYSTEM_KEY_MODIFIERS {
        let (message, covered) = refusal(&page(&format!("on-keydown.{word}.enter")));
        assert_eq!(covered, format!(".{word}"), "{message}");
        assert!(
            message.contains(&format!("`.{word}` waits for a modifier key")),
            "{message}"
        );
    }
}

#[test]
fn every_catalogued_qualifier_is_accepted_where_it_belongs() {
    for (word, _) in BEHAVIOUR_MODIFIERS {
        common::accepted(&page(&format!("on-click.{word}")));
    }
    for (word, _) in KEY_MODIFIERS {
        for event in KEY_EVENTS {
            common::accepted(&page(&format!("on-{event}.{word}")));
        }
    }
    common::accepted(&page("on-keydown.k.prevent"));
}

// ── no tier keeps a rule of its own ─────────────────────────────────────────────────────────────

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

#[path = "support/lgx_lower_source.rs"]
mod lgx_lower_source;

/// The sources that read a handler's qualifiers, besides the `lgx_lower` module (read whole by
/// [`tier_sources`]).
const TIER_SOURCES: &[&str] = &[
    "crates/logicaffeine_language/src/parser/component.rs",
    "crates/logicaffeine_language/src/lgx/template.rs",
    "crates/logicaffeine_language/src/ast/view.rs",
    "crates/logicaffeine_compile/src/interpreter.rs",
];

/// Every source that reads a handler's qualifiers, as `(label, text)`: the `lgx_lower` module
/// whole, then each of [`TIER_SOURCES`].
fn tier_sources() -> Vec<(String, String)> {
    let mut out = vec![("parser/lgx_lower/".to_string(), lgx_lower_source::source())];
    for rel in TIER_SOURCES {
        let text =
            std::fs::read_to_string(repo().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
        out.push((rel.to_string(), text));
    }
    out
}

/// Every quoted catalogued word in `text`, with its line.
fn quoted_catalogue_words(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        for w in EventModifier::every_word() {
            if line.contains(&format!("\"{w}\"")) {
                out.push((n + 1, w.to_string()));
            }
        }
    }
    out
}

#[test]
fn no_tier_spells_a_behavioural_or_system_qualifier_itself() {
    // A tier that compares a qualifier against its own string literal is a second copy of the
    // catalogue, and two copies are how `.prevnt` became a key on one tier and not another. Named
    // keys are ordinary words (`end`, `home`, `tab`) and are checked by the key census below
    // instead.
    let owned: Vec<&str> = BEHAVIOUR_MODIFIERS
        .iter()
        .map(|(w, _)| *w)
        .chain(SYSTEM_KEY_MODIFIERS.iter().map(|(w, _)| *w))
        .collect();
    for (rel, text) in tier_sources() {
        let hits: Vec<(usize, String)> = quoted_catalogue_words(&text)
            .into_iter()
            .filter(|(_, w)| owned.contains(&w.as_str()))
            .collect();
        assert!(
            hits.is_empty(),
            "{rel} spells event qualifiers itself at {hits:?}; read them through \
             `lgx::html::EventModifier` instead"
        );
    }
}

#[test]
fn the_source_census_sees_a_hand_written_comparison() {
    let text = "let x = 1;\n    .any(|m| interner.resolve(*m) == \"prevent\");\n";
    assert_eq!(
        quoted_catalogue_words(text),
        vec![(2, "prevent".to_string())]
    );
}

#[test]
fn no_tier_defines_its_own_modifier_rule() {
    for (rel, text) in tier_sources() {
        assert!(
            !text.contains("fn modifier_key"),
            "{rel} defines `modifier_key`, a second copy of the catalogue's `EventModifier::key_of`"
        );
    }
    // The lowering hands `.prevent` to the host as a listen bit, so it reads it through the
    // catalogue's `host_listen_bits`.
    let lowering = lgx_lower_source::source();
    assert!(
        lowering.contains("EventModifier::key_of")
            && lowering.contains("EventModifier::host_listen_bits"),
        "parser/lgx_lower/ must read a handler's key and `.prevent` through the catalogue"
    );
}

// ── every taught example uses words the catalogue knows ─────────────────────────────────────────

/// Every `on-<event>.<q>[.<q>…]` directive written in `text`.
fn directives_in(text: &str) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("on-") {
        let before_ok = i == 0
            || !rest[..i]
                .chars()
                .last()
                .is_some_and(|c| c.is_alphanumeric() || c == '-');
        let tail = &rest[i + 3..];
        let end = tail
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '-'))
            .unwrap_or(tail.len());
        let name = &tail[..end];
        let followed_by_value = tail[end..].starts_with("={");
        if before_ok && followed_by_value && name.contains('.') {
            let mut parts = name.split('.');
            let event = parts.next().unwrap_or("").to_string();
            let quals: Vec<String> = parts.map(str::to_string).collect();
            if !event.is_empty() && quals.iter().all(|q| !q.is_empty()) {
                out.push((event, quals));
            }
        }
        rest = &rest[i + 3..];
    }
    out
}

fn files_under(dir: &Path, exts: &[&str], out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name();
        if p.is_dir() {
            if name != "target" && name != "dist" && name != "node_modules" {
                files_under(&p, exts, out);
            }
        } else if p
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| exts.contains(&x))
        {
            out.push(p);
        }
    }
}

#[test]
fn every_taught_directive_classifies() {
    let root = repo();
    let mut files = Vec::new();
    files_under(&root.join("examples"), &["lg", "md"], &mut files);
    files_under(&root.join("docs"), &["md"], &mut files);
    files_under(
        &root.join("crates/logicaffeine_compile/assets/std"),
        &["lg", "md"],
        &mut files,
    );
    files_under(&root.join("apps/logicaffeine_web/src"), &["rs"], &mut files);
    files_under(
        &root.join("apps/logicaffeine_cli/templates"),
        &["lg", "md"],
        &mut files,
    );
    for extra in ["LOGOS_QUICKGUIDE.md", "AGENTS.md", "README.md"] {
        let p = root.join(extra);
        if p.is_file() {
            files.push(p);
        }
    }
    let mut checked = 0usize;
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap_or_default();
        for (event, quals) in directives_in(&text) {
            let words: Vec<&str> = quals.iter().map(String::as_str).collect();
            if let Err((_, e)) = classify_modifiers(&event, &words) {
                panic!(
                    "{} teaches `on-{event}.{}`, which the catalogue refuses: {}",
                    f.display(),
                    quals.join("."),
                    e.message()
                );
            }
            checked += 1;
        }
    }
    assert!(
        checked > 0,
        "the sweep found no directive with a qualifier — it proved nothing"
    );
}

#[test]
fn the_directive_scanner_reads_what_it_should() {
    let text = "<form on-submit.prevent={save}> <input on-keydown.enter.prevent={add}> \
                <p on-click={go}> ask-on-x.y={z} on-keyup.a={k}";
    let got = directives_in(text);
    assert_eq!(
        got,
        vec![
            ("submit".to_string(), vec!["prevent".to_string()]),
            (
                "keydown".to_string(),
                vec!["enter".to_string(), "prevent".to_string()]
            ),
            ("keyup".to_string(), vec!["a".to_string()]),
        ]
    );
}
