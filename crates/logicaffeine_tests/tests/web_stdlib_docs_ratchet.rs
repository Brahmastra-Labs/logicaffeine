//! Every entry point of the web standard library must be documented.
//!
//! The doc-example lock proves the examples that EXIST still compile, and says nothing about a
//! surface that was never written about at all. This is the other half. It reads the
//! standard-library modules themselves and insists every function they expose is named in the web
//! guides — `docs/web.md` (the server) or `docs/frontend.md` (the browser). A new helper added to
//! `http.lg`, `forms.lg` or `ui.lg` therefore fails here until somebody writes the sentence
//! explaining it: undocumented is the failure mode, not the default.
//!
//! Three modules are held by an explicit list instead. `postgres.lg`, `crypto.lg` and `session.lg`
//! are mostly protocol and arithmetic plumbing — byte readers, frame builders, compression rounds,
//! text scanners — that no program calls and no guide should list. For those, the PUBLIC entry
//! points are named here and must stay documented; the ratchet on them is against silently dropping
//! a documented surface, not against adding an internal helper.
//!
//! A name is not deleted to make this pass. If a function is internal, make it private in its
//! module and it stops being a public entry point; if it is real, document it.

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn read(rel: &str) -> String {
    let p = repo_root().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

/// The two web guides, read as one text: a name documented in either is documented.
fn web_guide() -> String {
    format!("{}\n{}", read("docs/web.md"), read("docs/frontend.md"))
}

/// Every `## To <name>` / `## To native <name>` a module declares.
fn declared_names(module: &str) -> Vec<String> {
    let src = read(&format!(
        "crates/logicaffeine_compile/assets/std/{module}.lg"
    ));
    let mut out = Vec::new();
    for line in src.lines() {
        let Some(rest) = line.strip_prefix("## To ") else {
            continue;
        };
        let rest = rest.strip_prefix("native ").unwrap_or(rest);
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() {
            out.push(name);
        }
    }
    out
}

fn assert_documented(names: &[String], module: &str) {
    let guide = web_guide();
    let missing: Vec<&str> = names
        .iter()
        .map(|s| s.as_str())
        .filter(|n| !guide.contains(*n))
        .collect();
    assert!(
        missing.is_empty(),
        "{} entry point(s) of `{module}.lg` are not named anywhere in docs/web.md or docs/frontend.md:\n  {}\n\
         A shipped surface nobody can read about is a surface nobody can use.",
        missing.len(),
        missing.join("\n  ")
    );
}

#[test]
fn every_socket_primitive_is_documented() {
    let names = declared_names("tcp");
    assert!(names.len() >= 10, "tcp.lg shrank unexpectedly: {names:?}");
    assert_documented(&names, "tcp");
}

#[test]
fn every_http_vocabulary_word_is_documented() {
    let names = declared_names("http");
    assert!(names.len() >= 14, "http.lg shrank unexpectedly: {names:?}");
    assert_documented(&names, "http");
}

#[test]
fn every_static_file_helper_is_documented() {
    let names = declared_names("static");
    assert!(names.len() >= 4, "static.lg shrank unexpectedly: {names:?}");
    assert_documented(&names, "static");
}

#[test]
fn every_browser_fetch_wrapper_is_documented() {
    let names = declared_names("web");
    assert!(names.len() >= 9, "web.lg shrank unexpectedly: {names:?}");
    assert_documented(&names, "web");
}

#[test]
fn every_form_validator_is_documented() {
    let names = declared_names("forms");
    assert!(names.len() >= 14, "forms.lg shrank unexpectedly: {names:?}");
    assert_documented(&names, "forms");
}

#[test]
fn every_list_view_helper_is_documented() {
    let names = declared_names("ui");
    assert!(names.len() >= 7, "ui.lg shrank unexpectedly: {names:?}");
    assert_documented(&names, "ui");
}

#[test]
fn the_public_session_surface_stays_documented() {
    let public = [
        "signSession",
        "verifySession",
        "sessionMac",
        "csrfToken",
        "csrfValid",
        "sessionSecret",
        "randomSecret",
        "hexText",
        "unhexText",
        "sessionField",
        "sessionInt",
        "sessionBool",
    ];
    let declared = declared_names("session");
    let guide = web_guide();
    for name in public {
        assert!(
            declared.iter().any(|d| d == name),
            "{name} is listed here as a public entry point but `session.lg` no longer declares it"
        );
        assert!(
            guide.contains(name),
            "the session entry point {name} left the web guides"
        );
    }
}

#[test]
fn the_public_database_surface_stays_documented() {
    // `postgres.lg` is ~86 functions, most of them wire plumbing. These are the
    // ones a program calls, and they must stay in the guide.
    let public = [
        "pgConnect",
        // The TLS half of opening a connection. A managed database refuses a cleartext one, so this
        // is the entry point a deployed program actually calls — listing it here keeps it in the
        // guide rather than letting the documented surface be the one nobody can use in production.
        "pgConnectSecure",
        "pgQuery",
        "pgQueryParams",
        "pgExecute",
        "pgClose",
        "pgPrepare",
        "pgExecPrepared",
        "pgTextAt",
        "pgIntAt",
        "pgBoolAt",
        "pgRealAt",
        "pgDecimalAt",
        "pgUuidAt",
        "pgEnumAt",
        "pgIsNullAt",
    ];
    let declared = declared_names("postgres");
    let guide = web_guide();
    for name in public {
        assert!(
            declared.iter().any(|d| d == name),
            "{name} is listed here as a public entry point but `postgres.lg` no longer \
             declares it — update this list deliberately, do not let it drift"
        );
        assert!(
            guide.contains(name),
            "the database entry point {name} left the web guides"
        );
    }
}

#[test]
fn the_public_digest_surface_stays_documented() {
    let public = ["sha256", "hmacSha256", "pbkdf2Sha256"];
    let declared = declared_names("crypto");
    let guide = web_guide();
    for name in public {
        assert!(
            declared.iter().any(|d| d == name),
            "{name} is listed here as a public entry point but `crypto.lg` no longer declares it"
        );
        assert!(
            guide.contains(name),
            "the digest entry point {name} left the web guides"
        );
    }
}

#[test]
fn the_strict_json_reader_stays_documented() {
    let public = [
        "jsonTryParse",
        "jsonParseStrict",
        "jsonString",
        "jsonInt",
        "jsonBool",
        "jsonArray",
        "jsonField",
        "jsonRequireObject",
        "jsonNoExtraKeys",
    ];
    let declared = declared_names("json");
    let guide = web_guide();
    for name in public {
        assert!(
            declared.iter().any(|d| d == name),
            "{name} is listed here as a public entry point but `json.lg` no longer declares it"
        );
        assert!(
            guide.contains(name),
            "the JSON entry point {name} left the web guides"
        );
    }
}

#[test]
fn the_guide_is_reachable_from_the_documentation_index() {
    // A guide nothing links to is a guide nobody finds.
    let index = read("docs/README.md");
    for guide in ["web.md", "frontend.md"] {
        assert!(
            index.contains(guide),
            "docs/README.md must list {guide} in its hand-maintained index"
        );
    }
}

/// Each web guide ends by naming the tests that pin the behavior it describes. Those names are a
/// promise — "do not take my word for it, go read this" — and a promise that points at a file which
/// no longer exists is worse than no promise at all.
///
/// This reads the names straight out of that section rather than from a list here, so it catches a
/// test that was renamed away AND a name written into a guide that never existed.
#[test]
fn every_test_the_guides_point_at_actually_exists() {
    for guide_path in ["docs/web.md", "docs/frontend.md"] {
        let guide = read(guide_path);
        let marker = "## Where this is tested";
        let section = guide
            .split(marker)
            .nth(1)
            .unwrap_or_else(|| panic!("{guide_path} must keep its `{marker}` section"));
        let section = section.split("\n## ").next().unwrap_or(section);

        // Every backticked token in that section is a test binary name.
        let mut named = Vec::new();
        let mut rest = section;
        while let Some(open) = rest.find('`') {
            rest = &rest[open + 1..];
            let Some(close) = rest.find('`') else { break };
            let token = &rest[..close];
            rest = &rest[close + 1..];
            if token.len() > 6
                && token
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            {
                named.push(token.to_string());
            }
        }
        assert!(
            named.len() >= 8,
            "{guide_path} should still cite the tests that pin it, found {}: {named:?}",
            named.len()
        );

        let root = repo_root();
        let mut missing = Vec::new();
        for name in &named {
            let found = ["crates", "apps"].iter().any(|top| {
                let dir = root.join(top);
                std::fs::read_dir(&dir)
                    .map(|entries| {
                        entries
                            .filter_map(|e| e.ok())
                            .any(|e| e.path().join("tests").join(format!("{name}.rs")).is_file())
                    })
                    .unwrap_or(false)
            });
            if !found {
                missing.push(name.clone());
            }
        }
        assert!(
            missing.is_empty(),
            "{guide_path} points readers at {} test(s) that do not exist:\n  {}\n\
             Rename the citation to match the tree, or delete the claim.",
            missing.len(),
            missing.join("\n  ")
        );
    }
}
