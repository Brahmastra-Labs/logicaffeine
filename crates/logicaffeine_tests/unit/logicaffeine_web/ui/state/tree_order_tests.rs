//! One order for the file tree: the tree the Studio loads from its file system and the tree it
//! builds from a list of paths sort the same way, so they can never disagree about which file
//! comes first.

use super::FileNode;

/// Each node's name, indented one step per level, in the order the tree shows them.
fn outline(node: &FileNode) -> Vec<String> {
    fn walk(node: &FileNode, depth: usize, out: &mut Vec<String>) {
        for child in &node.children {
            let mark = if child.is_directory { "/" } else { "" };
            out.push(format!("{}{}{mark}", "  ".repeat(depth), child.name));
            walk(child, depth + 1, out);
        }
    }
    let mut out = Vec::new();
    walk(node, 0, &mut out);
    out
}

#[test]
fn a_tree_built_from_paths_holds_each_file_under_its_folders() {
    let tree = FileNode::from_paths(["/examples/logic/quantifiers.logic", "/examples/code/basics/numbers.logos"]);
    assert_eq!(tree, {
        let mut numbers = FileNode::directory("basics".into(), "/examples/code/basics".into());
        numbers.children = vec![FileNode::file("numbers.logos".into(), "/examples/code/basics/numbers.logos".into())];
        let mut code = FileNode::directory("code".into(), "/examples/code".into());
        code.children = vec![numbers];
        let mut logic = FileNode::directory("logic".into(), "/examples/logic".into());
        logic.children = vec![FileNode::file("quantifiers.logic".into(), "/examples/logic/quantifiers.logic".into())];
        let mut examples = FileNode::directory("examples".into(), "/examples".into());
        examples.children = vec![code, logic];
        let mut root = FileNode::root();
        root.children = vec![examples];
        root
    });
}

#[test]
fn a_file_at_the_root_sits_beside_the_folders() {
    let tree = FileNode::from_paths(["/notes.logic", "/workspace/a.logos"]);
    assert_eq!(outline(&tree), ["workspace/", "  a.logos", "notes.logic"]);
    assert_eq!(tree.children[1].path, "/notes.logic");
}

#[test]
fn a_path_named_twice_is_one_file() {
    let tree = FileNode::from_paths(["/workspace/a.logos", "/workspace/a.logos"]);
    assert_eq!(outline(&tree), ["workspace/", "  a.logos"]);
}

#[test]
fn folders_come_first_and_code_categories_keep_their_learning_order() {
    let tree = FileNode::from_paths([
        "/examples/code/zeta.logos",
        "/examples/code/projects/p.logos",
        "/examples/code/aaa-new-category/x.logos",
        "/examples/code/strings/s.logos",
        "/examples/code/basics/b.logos",
        "/examples/code/time-and-money/t.logos",
        "/examples/code/alpha.logos",
    ]);
    assert_eq!(
        outline(&tree),
        [
            "examples/",
            "  code/",
            "    basics/",
            "      b.logos",
            "    strings/",
            "      s.logos",
            "    time-and-money/",
            "      t.logos",
            "    projects/",
            "      p.logos",
            "    aaa-new-category/",
            "      x.logos",
            "    alpha.logos",
            "    zeta.logos",
        ],
        "basics first, projects last of the curated set, a new category after them, then files"
    );
}

#[test]
fn names_sort_without_regard_to_case() {
    let tree = FileNode::from_paths(["/w/b.logos", "/w/A.logos", "/w/c.logos"]);
    assert_eq!(outline(&tree), ["w/", "  A.logos", "  b.logos", "  c.logos"]);
}

#[test]
fn sorting_one_level_puts_it_in_the_trees_order() {
    let mut dir = FileNode::directory("code".into(), "/examples/code".into());
    dir.children = vec![
        FileNode::file("hello-world.logos".into(), "/examples/code/hello-world.logos".into()),
        FileNode::directory("projects".into(), "/examples/code/projects".into()),
        FileNode::directory("web".into(), "/examples/code/web".into()),
        FileNode::directory("basics".into(), "/examples/code/basics".into()),
    ];
    dir.sort_children();
    let names: Vec<&str> = dir.children.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["basics", "projects", "web", "hello-world.logos"]);
}

/// Each folder's path and whether it is open, in the tree's order.
fn openness(node: &FileNode) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    if node.is_directory {
        out.push((node.path.clone(), node.expanded));
    }
    for child in &node.children {
        out.extend(openness(child));
    }
    out
}

#[test]
fn a_reloaded_tree_keeps_the_readers_open_and_closed_folders() {
    let mut shown = FileNode::from_paths(["/examples/code/basics/b.logos", "/examples/logic/q.logic"]);
    shown.find_mut("/examples/code").expect("a code folder").toggle_expanded();
    let reloaded = FileNode::from_paths(["/examples/code/basics/b.logos", "/examples/logic/q.logic", "/workspace/mine.logos"]);
    let kept = reloaded.with_expansion_of(&shown);
    assert_eq!(
        openness(&kept),
        [
            ("/".to_string(), true),
            ("/examples".to_string(), true),
            ("/examples/code".to_string(), false),
            ("/examples/code/basics".to_string(), true),
            ("/examples/logic".to_string(), true),
            ("/workspace".to_string(), true),
        ],
        "the folder the reader closed opened again, or a new folder did not start open"
    );
    assert_eq!(files(&kept), ["/examples/code/basics/b.logos", "/examples/logic/q.logic", "/workspace/mine.logos"]);
}

/// Every file the tree holds, in its order.
fn files(node: &FileNode) -> Vec<String> {
    if !node.is_directory {
        return vec![node.path.clone()];
    }
    node.children.iter().flat_map(files).collect()
}
