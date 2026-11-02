use super::*;
use std::path::{Path, PathBuf};

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for path in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display())).filter_map(|e| e.ok().map(|e| e.path())) {
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// The chevron is drawn in one place. A second copy of its path would let the file tree's
/// folders and the Studio's blocks drift apart.
#[test]
fn the_drawn_chevron_is_defined_once() {
    let shape = CHEVRON_SVG.split("d=\"").nth(1).and_then(|rest| rest.split('"').next()).expect("the chevron has a path");
    let mut files = Vec::new();
    sources(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    let copies: Vec<String> = files
        .iter()
        .flat_map(|file| {
            let text = std::fs::read_to_string(file).expect("read a source file");
            std::iter::repeat(file.display().to_string()).take(text.matches(shape).count())
        })
        .collect();
    assert_eq!(copies.len(), 1, "the chevron's path {shape:?} is drawn {} times: {copies:?}", copies.len());
}

fn render(open: bool) -> String {
    #[derive(Props, Clone, PartialEq)]
    struct HostProps {
        open: bool,
    }
    #[allow(non_snake_case)]
    fn Host(props: HostProps) -> Element {
        rsx! { Chevron { open: props.open } }
    }
    let mut vdom = VirtualDom::new_with_props(Host, HostProps { open });
    vdom.rebuild_in_place();
    dioxus_ssr::render(&vdom)
}

#[test]
fn a_closed_chevron_is_drawn_unturned() {
    let html = render(false);
    assert!(html.contains(r#"class="chevron""#) && html.contains(CHEVRON_SVG), "{html}");
}

#[test]
fn an_open_chevron_takes_the_turn() {
    let html = render(true);
    assert!(html.contains(r#"class="chevron open""#) && html.contains(CHEVRON_SVG), "{html}");
    assert!(CHEVRON_STYLE.contains(".chevron.open {\n    transform: rotate(90deg);"), "the open chevron does not turn a quarter");
}
