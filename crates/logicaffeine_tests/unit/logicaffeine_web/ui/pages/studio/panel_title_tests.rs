use std::path::Path;

/// The CSS in a source file: the text of its raw strings, comments dropped.
fn stylesheets(source: &str) -> String {
    let css: String = source.split("r#\"").skip(1).filter_map(|rest| rest.split("\"#").next()).collect::<Vec<_>>().join("\n");
    let mut out = String::new();
    let mut rest = css.as_str();
    while let Some(at) = rest.find("/*") {
        out.push_str(&rest[..at]);
        rest = rest[at..].find("*/").map_or("", |end| &rest[at + end + 2..]);
    }
    out.push_str(rest);
    out
}

/// The selectors of every rule in `css` that declares `declaration`.
fn rules_declaring(css: &str, declaration: &str) -> Vec<Vec<String>> {
    css.match_indices(declaration)
        .map(|(at, _)| {
            let open = css[..at].rfind('{').expect("a declaration sits inside a rule");
            let start = css[..open].rfind(['}', '{']).map_or(0, |i| i + 1);
            css[start..open].split(',').map(|s| s.split_whitespace().collect::<Vec<_>>().join(" ")).collect()
        })
        .collect()
}

/// The Studio's titles — every panel header, the Files header and each collapsible block's
/// heading — are set in one type, declared by one rule. Its `letter-spacing: 0.3px` is the
/// type's mark: a second rule declaring it is a copy that can drift from the first.
#[test]
fn the_panel_title_type_is_defined_once() {
    let web = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui");
    let mut files = vec![web.join("pages/studio.rs")];
    let mut components: Vec<_> = std::fs::read_dir(web.join("components"))
        .expect("read the components")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "rs"))
        .collect();
    components.sort();
    files.extend(components);
    let rules: Vec<(String, Vec<String>)> = files
        .iter()
        .flat_map(|file| {
            let css = stylesheets(&std::fs::read_to_string(file).expect("read a Studio source"));
            let name = file.file_name().unwrap().to_string_lossy().into_owned();
            rules_declaring(&css, "letter-spacing: 0.3px").into_iter().map(move |selectors| (name.clone(), selectors))
        })
        .collect();
    assert_eq!(rules.len(), 1, "the panel-title type is declared by {} rules: {rules:?}", rules.len());
    let (file, selectors) = &rules[0];
    assert_eq!(file, "studio.rs", "the panel-title type lives in {file}, not beside the Studio's tokens");
    for wanted in [".studio-panel .panel-header", ".panel-title"] {
        assert!(selectors.iter().any(|s| s == wanted), "the panel-title rule does not set {wanted}: {selectors:?}");
    }
}

#[test]
fn a_rule_is_found_by_what_it_declares() {
    let css = ".a { color: red; }\n@media (x) {\n    .b,\n    .c { letter-spacing: 0.3px; }\n}";
    assert_eq!(rules_declaring(css, "letter-spacing: 0.3px"), vec![vec![".b".to_string(), ".c".to_string()]]);
}

#[test]
fn a_comment_is_not_css() {
    assert_eq!(stylesheets("const S: &str = r#\"/* .x { letter-spacing: 0.3px; } */ .y { }\"#;").trim(), ".y { }");
}
