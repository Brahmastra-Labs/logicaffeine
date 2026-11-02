//! The Studio's files: the shipped examples served read-only from the site, over the reader's store.
//!
//! The store here is an in-memory stand-in for the browser's OPFS that counts every call made to
//! it, so each test can say exactly what a visit asks of the reader's storage. Before the example
//! layer, every visit wrote all ~160 examples (and removed ~28 retired ones) before the file tree
//! or the opening file could load.

use super::*;
use crate::ui::examples::{ALL_CODE_EXAMPLES, ALL_HARDWARE_SPECS, ALL_LOGIC_EXAMPLES, ALL_MATH_EXAMPLES};
use crate::ui::pages::studio_opening::StudioOpening;
use logicaffeine_system::fs::VfsError;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

/// The first Code example the file tree shows today: the file a bare `/studio` opens.
const FIRST_CODE_EXAMPLE: &str = "/examples/code/basics/control-flow.logos";

fn every_example() -> impl Iterator<Item = &'static ExampleSpec> {
    ALL_LOGIC_EXAMPLES.iter().chain(ALL_CODE_EXAMPLES).chain(ALL_MATH_EXAMPLES).chain(ALL_HARDWARE_SPECS)
}

fn source(path: &str) -> &'static str {
    every_example().find(|spec| spec.vfs_path == path).unwrap_or_else(|| panic!("no shipped example at {path}")).source
}

fn run<F: std::future::Future>(future: F) -> F::Output {
    logicaffeine_system::tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a Tokio runtime")
        .block_on(future)
}

/// An in-memory store with the browser OPFS's rules: writing a file makes its folders, a folder
/// can be removed only once it is empty, and a missing path is `NotFound`. It counts every call.
#[derive(Default)]
struct Store {
    files: Mutex<BTreeMap<String, Vec<u8>>>,
    dirs: Mutex<BTreeSet<String>>,
    calls: Mutex<BTreeMap<&'static str, usize>>,
    /// How many more removes succeed before the store fails one, as a closed tab interrupts a
    /// removal halfway; `None` is no limit.
    removes_left: Mutex<Option<usize>>,
}

fn normal(path: &str) -> String {
    format!("/{}", path.trim_matches('/'))
}

fn parent(path: &str) -> String {
    match path.rsplit_once('/') {
        Some(("", _)) | None => "/".to_string(),
        Some((up, _)) => up.to_string(),
    }
}

impl Store {
    /// A store holding `files` (path, text) and their folders, as an earlier visit left it.
    fn holding(files: &[(&str, &str)]) -> Self {
        let store = Store::default();
        for (path, text) in files {
            store.put(path, text);
        }
        store
    }

    fn put(&self, path: &str, text: &str) {
        let path = normal(path);
        self.make_dirs(&parent(&path));
        self.files.lock().unwrap().insert(path, text.as_bytes().to_vec());
    }

    fn make_dirs(&self, dir: &str) {
        let mut dir = normal(dir);
        let mut dirs = self.dirs.lock().unwrap();
        while dir != "/" {
            dirs.insert(dir.clone());
            dir = parent(&dir);
        }
    }

    fn count(&self, call: &'static str) {
        *self.calls.lock().unwrap().entry(call).or_default() += 1;
    }

    fn calls(&self, call: &str) -> usize {
        self.calls.lock().unwrap().get(call).copied().unwrap_or(0)
    }

    /// Every call that changes the store.
    fn changes(&self) -> usize {
        ["write", "append", "remove", "create_dir_all", "rename"].iter().map(|call| self.calls(call)).sum()
    }

    fn all_calls(&self) -> usize {
        self.calls.lock().unwrap().values().sum()
    }

    fn paths(&self) -> Vec<String> {
        let mut paths: Vec<String> = self.files.lock().unwrap().keys().cloned().collect();
        paths.extend(self.dirs.lock().unwrap().iter().map(|dir| format!("{dir}/")));
        paths.sort();
        paths
    }

    fn text(&self, path: &str) -> Option<String> {
        self.files.lock().unwrap().get(&normal(path)).map(|bytes| String::from_utf8_lossy(bytes).into_owned())
    }
}

impl Vfs for Store {
    fn read<'a, 'b, 'f>(&'a self, path: &'b str) -> Answer<'f, Vec<u8>>
    where
        'a: 'f,
        'b: 'f,
        Self: 'f,
    {
        Box::pin(async move {
            self.count("read");
            self.files.lock().unwrap().get(&normal(path)).cloned().ok_or_else(|| VfsError::NotFound(path.to_string()))
        })
    }

    fn read_to_string<'a, 'b, 'f>(&'a self, path: &'b str) -> Answer<'f, String>
    where
        'a: 'f,
        'b: 'f,
        Self: 'f,
    {
        Box::pin(async move {
            self.count("read_to_string");
            self.text(path).ok_or_else(|| VfsError::NotFound(path.to_string()))
        })
    }

    fn write<'a, 'b, 'c, 'f>(&'a self, path: &'b str, contents: &'c [u8]) -> Answer<'f, ()>
    where
        'a: 'f,
        'b: 'f,
        'c: 'f,
        Self: 'f,
    {
        Box::pin(async move {
            self.count("write");
            let path = normal(path);
            self.make_dirs(&parent(&path));
            self.files.lock().unwrap().insert(path, contents.to_vec());
            Ok(())
        })
    }

    fn append<'a, 'b, 'c, 'f>(&'a self, path: &'b str, contents: &'c [u8]) -> Answer<'f, ()>
    where
        'a: 'f,
        'b: 'f,
        'c: 'f,
        Self: 'f,
    {
        Box::pin(async move {
            self.count("append");
            let path = normal(path);
            self.make_dirs(&parent(&path));
            self.files.lock().unwrap().entry(path).or_default().extend_from_slice(contents);
            Ok(())
        })
    }

    fn exists<'a, 'b, 'f>(&'a self, path: &'b str) -> Answer<'f, bool>
    where
        'a: 'f,
        'b: 'f,
        Self: 'f,
    {
        Box::pin(async move {
            self.count("exists");
            let path = normal(path);
            Ok(path == "/" || self.files.lock().unwrap().contains_key(&path) || self.dirs.lock().unwrap().contains(&path))
        })
    }

    fn remove<'a, 'b, 'f>(&'a self, path: &'b str) -> Answer<'f, ()>
    where
        'a: 'f,
        'b: 'f,
        Self: 'f,
    {
        Box::pin(async move {
            self.count("remove");
            {
                let mut left = self.removes_left.lock().unwrap();
                match *left {
                    Some(0) => return Err(VfsError::IoError(std::io::Error::other("the removal was interrupted"))),
                    Some(n) => *left = Some(n - 1),
                    None => {}
                }
            }
            let path = normal(path);
            if self.files.lock().unwrap().remove(&path).is_some() {
                return Ok(());
            }
            if !self.dirs.lock().unwrap().contains(&path) {
                return Err(VfsError::NotFound(path));
            }
            let inside = format!("{path}/");
            let occupied = self.files.lock().unwrap().keys().any(|f| f.starts_with(&inside))
                || self.dirs.lock().unwrap().iter().any(|d| d.starts_with(&inside));
            if occupied {
                return Err(VfsError::IoError(std::io::Error::other(format!("{path} is not empty"))));
            }
            self.dirs.lock().unwrap().remove(&path);
            Ok(())
        })
    }

    fn create_dir_all<'a, 'b, 'f>(&'a self, path: &'b str) -> Answer<'f, ()>
    where
        'a: 'f,
        'b: 'f,
        Self: 'f,
    {
        Box::pin(async move {
            self.count("create_dir_all");
            self.make_dirs(path);
            Ok(())
        })
    }

    fn rename<'a, 'b, 'c, 'f>(&'a self, from: &'b str, to: &'c str) -> Answer<'f, ()>
    where
        'a: 'f,
        'b: 'f,
        'c: 'f,
        Self: 'f,
    {
        Box::pin(async move {
            self.count("rename");
            let bytes = self.files.lock().unwrap().remove(&normal(from)).ok_or_else(|| VfsError::NotFound(from.to_string()))?;
            let to = normal(to);
            self.make_dirs(&parent(&to));
            self.files.lock().unwrap().insert(to, bytes);
            Ok(())
        })
    }

    fn list_dir<'a, 'b, 'f>(&'a self, path: &'b str) -> Answer<'f, Vec<DirEntry>>
    where
        'a: 'f,
        'b: 'f,
        Self: 'f,
    {
        Box::pin(async move {
            self.count("list_dir");
            let dir = normal(path);
            if dir != "/" && !self.dirs.lock().unwrap().contains(&dir) {
                return Err(VfsError::NotFound(dir));
            }
            let mut entries: Vec<DirEntry> = self
                .dirs
                .lock()
                .unwrap()
                .iter()
                .filter(|d| parent(d) == dir)
                .map(|d| DirEntry { name: d.rsplit('/').next().unwrap().to_string(), is_directory: true })
                .collect();
            entries.extend(
                self.files
                    .lock()
                    .unwrap()
                    .keys()
                    .filter(|f| parent(f) == dir)
                    .map(|f| DirEntry { name: f.rsplit('/').next().unwrap().to_string(), is_directory: false }),
            );
            Ok(entries)
        })
    }
}

/// The names `path` lists, sorted.
fn names(files: &StudioFiles<Store>, path: &str) -> Vec<String> {
    let mut names: Vec<String> = run(files.list_dir(path)).unwrap_or_else(|e| panic!("listing {path}: {e}")).into_iter().map(|e| e.name).collect();
    names.sort();
    names
}

/// Every file `node` holds, in the tree's order.
fn files_of(node: &FileNode) -> Vec<String> {
    if !node.is_directory {
        return vec![node.path.clone()];
    }
    node.children.iter().flat_map(files_of).collect()
}

fn loaded(files: &StudioFiles<Store>) -> FileNode {
    let mut tree = FileNode::root();
    run(load_dir_recursive(files, "/", &mut tree)).expect("the tree loads");
    tree
}

// ---------------------------------------------------------------------------------------------
// The example layer
// ---------------------------------------------------------------------------------------------

#[test]
fn an_example_reads_from_the_site_without_asking_the_store() {
    let files = StudioFiles::new(Store::default());
    assert_eq!(run(files.read_to_string(FIRST_CODE_EXAMPLE)).ok().as_deref(), Some(source(FIRST_CODE_EXAMPLE)));
    assert_eq!(run(files.read(FIRST_CODE_EXAMPLE)).ok(), Some(source(FIRST_CODE_EXAMPLE).as_bytes().to_vec()));
    assert_eq!(files.store().all_calls(), 0, "reading an example asked the store");
}

#[test]
fn an_example_reads_the_same_with_or_without_its_leading_slash() {
    let files = StudioFiles::new(Store::default());
    let bare = FIRST_CODE_EXAMPLE.trim_start_matches('/');
    assert_eq!(run(files.read_to_string(bare)).ok().as_deref(), Some(source(FIRST_CODE_EXAMPLE)));
}

#[test]
fn the_shipped_example_wins_over_an_old_copy() {
    let files = StudioFiles::new(Store::holding(&[(FIRST_CODE_EXAMPLE, "an old copy")]));
    assert_eq!(run(files.read_to_string(FIRST_CODE_EXAMPLE)).ok().as_deref(), Some(source(FIRST_CODE_EXAMPLE)));
}

#[test]
fn a_retired_example_is_not_found_even_where_an_old_copy_lingers() {
    let files = StudioFiles::new(Store::holding(&[("/examples/code/fibonacci.logos", "retired")]));
    assert!(matches!(run(files.read_to_string("/examples/code/fibonacci.logos")), Err(VfsError::NotFound(_))));
    assert!(!run(files.exists("/examples/code/fibonacci.logos")).unwrap());
}

#[test]
fn an_example_is_read_only() {
    let files = StudioFiles::new(Store::default());
    let refused = |result: VfsResult<()>, what: &str| match result {
        Err(VfsError::PermissionDenied(why)) => {
            assert!(why.contains("read-only"), "{what}: {why}");
            assert!(!why.contains(';') && !why.contains('\u{2014}'), "{what} reads {why:?}, with a semicolon or an em dash");
        }
        other => panic!("{what} was not refused: {other:?}"),
    };
    refused(run(files.write(FIRST_CODE_EXAMPLE, b"mine")), "writing an example");
    refused(run(files.append(FIRST_CODE_EXAMPLE, b"mine")), "appending to an example");
    refused(run(files.remove(FIRST_CODE_EXAMPLE)), "removing an example");
    refused(run(files.rename(FIRST_CODE_EXAMPLE, "/workspace/mine.logos")), "moving an example away");
    refused(run(files.rename("/workspace/mine.logos", "/examples/code/mine.logos")), "moving a file into the examples");
    refused(run(files.write("/examples/code/new.logos", b"mine")), "writing a new file among the examples");
    refused(run(files.create_dir_all("/examples/code/new-folder")), "making a folder among the examples");
    assert_eq!(files.store().changes(), 0, "a refused change reached the store");
    assert_eq!(run(files.read_to_string(FIRST_CODE_EXAMPLE)).ok().as_deref(), Some(source(FIRST_CODE_EXAMPLE)));
}

#[test]
fn an_example_folder_exists_and_lists_its_examples() {
    let files = StudioFiles::new(Store::default());
    assert!(run(files.exists("/examples")).unwrap());
    assert!(run(files.exists("/examples/code/basics")).unwrap());
    assert!(run(files.exists(FIRST_CODE_EXAMPLE)).unwrap());
    let basics: Vec<String> = {
        let mut names: Vec<String> = every_example()
            .filter_map(|spec| spec.vfs_path.strip_prefix("/examples/code/basics/"))
            .map(str::to_string)
            .collect();
        names.sort();
        names
    };
    assert_eq!(names(&files, "/examples/code/basics"), basics);
    assert_eq!(names(&files, "/examples"), ["code", "hardware", "logic", "math"]);
    assert!(matches!(run(files.list_dir("/examples/no-such-folder")), Err(VfsError::NotFound(_))));
    assert_eq!(files.store().all_calls(), 0, "the example folders asked the store");
}

#[test]
fn making_an_example_folder_that_exists_changes_nothing() {
    let files = StudioFiles::new(Store::default());
    assert!(run(files.create_dir_all("/examples/code")).is_ok());
    assert_eq!(files.store().all_calls(), 0);
}

#[test]
fn the_root_lists_the_examples_once_beside_the_readers_folders() {
    let fresh = StudioFiles::new(Store::holding(&[("/workspace/mine.logos", "Show 1.")]));
    assert_eq!(names(&fresh, "/"), ["examples", "workspace"]);
    let upgraded = StudioFiles::new(Store::holding(&[("/workspace/mine.logos", "Show 1."), (FIRST_CODE_EXAMPLE, "an old copy")]));
    assert_eq!(names(&upgraded, "/"), ["examples", "workspace"], "an old copy's folder shows twice");
    assert_eq!(names(&upgraded, "/examples/code/basics"), names(&fresh, "/examples/code/basics"), "an old copy shows through");
}

#[test]
fn the_readers_files_go_to_the_store() {
    let files = StudioFiles::new(Store::default());
    run(files.create_dir_all("/workspace")).unwrap();
    run(files.write("/workspace/mine.logos", b"Show 1.")).unwrap();
    assert_eq!(files.store().text("/workspace/mine.logos").as_deref(), Some("Show 1."));
    assert_eq!(run(files.read_to_string("/workspace/mine.logos")).ok().as_deref(), Some("Show 1."));
    assert!(run(files.exists("/workspace/mine.logos")).unwrap());
    run(files.rename("/workspace/mine.logos", "/workspace/renamed.logos")).unwrap();
    assert_eq!(files.store().text("/workspace/renamed.logos").as_deref(), Some("Show 1."));
}

// ---------------------------------------------------------------------------------------------
// The file tree
// ---------------------------------------------------------------------------------------------

#[test]
fn the_examples_part_of_the_tree_is_the_registry() {
    let files = StudioFiles::new(Store::default());
    let tree = loaded(&files);
    assert_eq!(tree, examples_tree(), "the tree the Studio loads is not the tree it shows on its first frame");
    assert_eq!(examples_tree(), FileNode::from_paths(every_example().map(|spec| spec.vfs_path)));
}

#[test]
fn the_layer_holds_exactly_the_registry() {
    let shown: BTreeSet<String> = files_of(&loaded(&StudioFiles::new(Store::default()))).into_iter().collect();
    let registry: BTreeSet<String> = every_example().map(|spec| spec.vfs_path.to_string()).collect();
    assert_eq!(shown, registry);
}

#[test]
fn no_example_folder_is_empty() {
    fn empty(node: &FileNode, out: &mut Vec<String>) {
        if node.is_directory {
            if node.children.is_empty() {
                out.push(node.path.clone());
            }
            node.children.iter().for_each(|child| empty(child, out));
        }
    }
    let mut found = Vec::new();
    empty(&examples_tree(), &mut found);
    assert!(found.is_empty(), "empty example folders render as empty nodes in the file tree: {found:?}");
}

#[test]
fn the_first_example_is_the_first_code_file_the_tree_shows() {
    let opening = StudioOpening::first_example();
    assert_eq!(examples_tree().first_file_under("/examples/code"), Some(opening.path.as_str()));
    assert_eq!(opening.path, FIRST_CODE_EXAMPLE);
    assert_eq!(opening.text, example_text(&opening.path));
}

#[test]
fn every_file_the_tree_shows_opens_with_the_text_the_files_hold() {
    let files = StudioFiles::new(Store::default());
    let shown = files_of(&loaded(&files));
    assert_eq!(shown.len(), every_example().count(), "the tree is missing examples");
    let differ: Vec<&String> = shown
        .iter()
        .filter(|path| StudioOpening::for_route(Some(path.as_str())).text != run(files.read_to_string(path)).ok().as_deref())
        .collect();
    assert!(differ.is_empty(), "a deep link to these opens with other text than the files hold: {differ:?}");
}

#[test]
fn the_readers_files_join_the_examples_in_the_tree() {
    let files = StudioFiles::new(Store::holding(&[("/workspace/mine.logos", "Show 1."), ("/workspace/notes/plan.logic", "Every cat sleeps.")]));
    let tree = loaded(&files);
    let names: Vec<&str> = tree.children.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["examples", "workspace"]);
    assert_eq!(tree.children[0], examples_tree().children[0], "the examples' part changed when the reader's files joined");
    assert_eq!(files_of(&tree.children[1]), ["/workspace/notes/plan.logic", "/workspace/mine.logos"]);
}

// ---------------------------------------------------------------------------------------------
// What opening the Studio asks of the store
// ---------------------------------------------------------------------------------------------

#[test]
fn opening_the_studio_writes_nothing() {
    let files = StudioFiles::new(Store::default());
    let (tree, read) = run(open_store(&files, FIRST_CODE_EXAMPLE));
    assert_eq!(tree.ok(), Some(examples_tree()));
    assert_eq!(read.as_deref(), Some(source(FIRST_CODE_EXAMPLE)));
    let store = files.store();
    assert_eq!(
        (store.calls("write"), store.calls("remove"), store.calls("create_dir_all"), store.calls("append"), store.calls("rename")),
        (0, 0, 0, 0, 0),
        "a first visit changed the store"
    );
}

#[test]
fn a_second_visit_writes_nothing_either() {
    let files = StudioFiles::new(Store::default());
    run(open_store(&files, FIRST_CODE_EXAMPLE));
    run(open_store(&files, FIRST_CODE_EXAMPLE));
    assert_eq!(files.store().changes(), 0, "two visits changed the store {} time(s)", files.store().changes());
}

#[test]
fn a_changed_example_is_served_without_a_write() {
    let files = StudioFiles::new(Store::holding(&[(FIRST_CODE_EXAMPLE, "the text an earlier build shipped")]));
    let (_, read) = run(open_store(&files, FIRST_CODE_EXAMPLE));
    assert_eq!(read.as_deref(), Some(source(FIRST_CODE_EXAMPLE)));
    assert_eq!(files.store().changes(), 0);
}

#[test]
fn a_readers_file_opens_from_the_store() {
    let files = StudioFiles::new(Store::holding(&[("/workspace/mine.logos", "Show 1.")]));
    let (tree, read) = run(open_store(&files, "/workspace/mine.logos"));
    assert_eq!(read.as_deref(), Some("Show 1."));
    assert!(files_of(&tree.unwrap()).contains(&"/workspace/mine.logos".to_string()));
    assert_eq!(files.store().changes(), 0);
}

#[test]
fn a_missing_file_reads_as_none() {
    let files = StudioFiles::new(Store::default());
    let (_, read) = run(open_store(&files, "/workspace/gone.logos"));
    assert_eq!(read, None);
    assert_eq!(files.store().changes(), 0);
}

// ---------------------------------------------------------------------------------------------
// Retiring the copies an earlier Studio wrote
// ---------------------------------------------------------------------------------------------

/// A store as an earlier Studio left it: copies of examples (one current, one retired, one in a
/// folder of its own), an empty example folder, and the reader's workspace.
fn upgraded_store() -> Store {
    let store = Store::holding(&[
        (FIRST_CODE_EXAMPLE, "an old copy"),
        ("/examples/code/fibonacci.logos", "retired"),
        ("/examples/logic/quantifiers.logic", "an old copy"),
        ("/workspace/mine.logos", "Show 1."),
        ("/workspace/notes/plan.logic", "Every cat sleeps."),
    ]);
    store.make_dirs("/examples/code/native");
    store
}

const WORKSPACE: [&str; 4] = ["/workspace/", "/workspace/mine.logos", "/workspace/notes/", "/workspace/notes/plan.logic"];

fn workspace_of(store: &Store) -> Vec<String> {
    store.paths().into_iter().filter(|p| p.starts_with("/workspace/")).collect()
}

#[test]
fn old_copies_are_retired_and_the_workspace_kept() {
    let files = StudioFiles::new(upgraded_store());
    let removed = run(files.retire_old_copies()).expect("the old copies are removed");
    let left: Vec<String> = files.store().paths().into_iter().filter(|p| p.starts_with("/examples")).collect();
    assert!(left.is_empty(), "old copies were left behind: {left:?}");
    // Three files and five folders: /examples, /examples/code, its basics and native, /examples/logic.
    assert_eq!(removed, 3 + 5, "the retirement removed {removed} entries");
    assert_eq!(workspace_of(files.store()), WORKSPACE);
    assert_eq!(files.store().text("/workspace/mine.logos").as_deref(), Some("Show 1."));
}

#[test]
fn retiring_again_finds_nothing_to_do() {
    let files = StudioFiles::new(upgraded_store());
    assert!(run(files.retire_old_copies()).unwrap() > 0, "the first retirement removed nothing");
    let before = files.store().calls("remove");
    assert_eq!(run(files.retire_old_copies()).unwrap(), 0);
    assert_eq!(files.store().calls("remove"), before, "a second retirement removed something");
    assert_eq!(workspace_of(files.store()), WORKSPACE);
}

#[test]
fn an_interrupted_retirement_finishes_on_the_next_visit() {
    let files = StudioFiles::new(upgraded_store());
    *files.store().removes_left.lock().unwrap() = Some(2);
    assert!(run(files.retire_old_copies()).is_err(), "the interrupted retirement claimed to finish");
    assert!(files.store().paths().iter().any(|p| p.starts_with("/examples")), "the interruption removed everything anyway");
    *files.store().removes_left.lock().unwrap() = None;
    run(files.retire_old_copies()).expect("the next visit finishes the retirement");
    assert!(!files.store().paths().iter().any(|p| p.starts_with("/examples")));
    assert_eq!(workspace_of(files.store()), WORKSPACE);
}

#[test]
fn a_store_that_never_held_examples_is_left_alone() {
    let files = StudioFiles::new(Store::holding(&[("/workspace/mine.logos", "Show 1.")]));
    assert_eq!(run(files.retire_old_copies()).unwrap(), 0);
    assert_eq!(files.store().changes(), 0);
    assert_eq!(workspace_of(files.store()), ["/workspace/", "/workspace/mine.logos"]);
}

// ---------------------------------------------------------------------------------------------
// A program's files: the Run path reads and writes through the Studio's files
// ---------------------------------------------------------------------------------------------

/// Run `program` in the Studio's interpreter with `vfs` as its files; what it printed, and its error.
fn run_program(program: &str, vfs: std::sync::Arc<dyn Vfs>) -> (String, Option<String>) {
    let quiet = std::rc::Rc::new(std::cell::RefCell::new(|_: String| {}));
    let result = run(logicaffeine_compile::interpret_streaming_with_vfs(program, quiet, Some(vfs)));
    (result.lines.join("\n"), result.error)
}

#[test]
fn a_program_reads_an_example_through_the_studios_files() {
    let program = format!("## Main\nRead text from file \"{FIRST_CODE_EXAMPLE}\".\nShow text.\n");
    let (printed, error) = run_program(&program, std::sync::Arc::new(StudioFiles::new(Store::default())));
    assert_eq!(error, None, "reading an example through the Studio's files failed");
    assert!(printed.contains("# Control Flow"), "the program printed {printed:?}, not the example");
    let (_, bare) = run_program(&program, std::sync::Arc::new(Store::default()));
    assert!(bare.is_some(), "the reader's store alone holds the example, so this proves nothing");
}

#[test]
fn a_program_writes_to_the_workspace_and_never_over_an_example() {
    let files = std::sync::Arc::new(StudioFiles::new(Store::default()));
    let (_, refused) = run_program(&format!("## Main\nWrite \"mine\" to file \"{FIRST_CODE_EXAMPLE}\".\n"), files.clone());
    let refused = refused.expect("writing over a shipped example succeeded");
    assert!(refused.contains("read-only"), "the refusal does not say why: {refused}");
    let (_, error) = run_program("## Main\nWrite \"mine\" to file \"/workspace/out.txt\".\n", files.clone());
    assert_eq!(error, None);
    assert_eq!(files.store().text("/workspace/out.txt").as_deref(), Some("mine"));
    assert_eq!(run(files.read_to_string(FIRST_CODE_EXAMPLE)).ok().as_deref(), Some(source(FIRST_CODE_EXAMPLE)));
}
