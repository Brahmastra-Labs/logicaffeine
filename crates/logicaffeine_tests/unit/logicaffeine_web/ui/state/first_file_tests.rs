use super::FileNode;

fn tree() -> FileNode {
    let file = |dir: &str, name: &str| FileNode::file(name.to_string(), format!("{dir}/{name}"));
    let mut basics = FileNode::directory("basics".into(), "/examples/code/basics".into());
    basics.children = vec![file("/examples/code/basics", "control-flow.logos"), file("/examples/code/basics", "fizzbuzz.logos")];
    let mut code = FileNode::directory("code".into(), "/examples/code".into());
    code.children = vec![basics];
    let mut logic = FileNode::directory("logic".into(), "/examples/logic".into());
    logic.children = vec![file("/examples/logic", "prover-demo.logic")];
    let mut examples = FileNode::directory("examples".into(), "/examples".into());
    examples.children = vec![code, logic];
    let mut root = FileNode::root();
    root.children = vec![examples];
    root
}

#[test]
fn the_first_file_under_a_directory_is_the_first_the_tree_shows() {
    assert_eq!(tree().first_file_under("/examples/code"), Some("/examples/code/basics/control-flow.logos"));
    assert_eq!(tree().first_file_under("/examples/logic"), Some("/examples/logic/prover-demo.logic"));
}

#[test]
fn a_directory_with_no_files_has_no_first_file() {
    assert_eq!(tree().first_file_under("/examples/math"), None);
}

#[test]
fn a_sibling_that_only_shares_a_prefix_is_not_under_the_directory() {
    let mut root = tree();
    let mut codex = FileNode::directory("codex".into(), "/examples/codex".into());
    codex.children = vec![FileNode::file("a.logos".into(), "/examples/codex/a.logos".into())];
    root.children[0].children.insert(0, codex);
    assert_eq!(root.first_file_under("/examples/code"), Some("/examples/code/basics/control-flow.logos"));
}
