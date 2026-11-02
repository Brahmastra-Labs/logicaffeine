/// An inline editor carries no stylesheet of its own — its host places the one stylesheet
/// ahead of the host's markup — so every file that sets an editor into its markup places
/// `InlineEditorStyle {}` too. A host that forgot would draw its editors unstyled.
#[test]
fn every_host_places_the_inline_editor_stylesheet() {
    fn sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("the source tree is readable") {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                sources(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&root, &mut files);
    let hosts: Vec<(String, bool)> = files
        .iter()
        .filter(|path| !path.ends_with("components/inline_editor.rs"))
        .filter_map(|path| {
            let text = std::fs::read_to_string(path).expect("a source file is readable");
            let name = path.strip_prefix(&root).unwrap_or(path).display().to_string();
            text.contains("InlineEditor {").then(|| (name, text.contains("InlineEditorStyle {}")))
        })
        .collect();
    assert!(hosts.len() >= 3, "only {} file(s) set an inline editor into their markup: {hosts:?}", hosts.len());
    let bare: Vec<&str> = hosts.iter().filter(|(_, styled)| !styled).map(|(name, _)| name.as_str()).collect();
    assert!(bare.is_empty(), "these files set an inline editor into their markup without its stylesheet: {bare:?}");
}
