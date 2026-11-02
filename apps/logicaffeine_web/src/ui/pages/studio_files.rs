//! The Studio's files: the shipped examples, read-only and served from the site itself, over the
//! reader's own store (the browser's OPFS, or IndexedDB in a private window).
//!
//! Every shipped example is compiled into the site, so the Studio never copies one into the
//! reader's store. `/examples` is a read-only layer answered from memory, and every other path is
//! the reader's. Opening the Studio writes nothing, the examples' part of the file tree exists
//! before any store answers ([`examples_tree`]), and a program still reads an example by its path.
//! Copies an earlier Studio wrote under `/examples` never show, and
//! [`StudioFiles::retire_old_copies`] removes them in the background.

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;

use crate::ui::examples::{seeded_examples, ExampleSpec};
use crate::ui::state::FileNode;
use logicaffeine_system::fs::{DirEntry, Vfs, VfsError, VfsResult};

/// Where the shipped examples live.
pub const EXAMPLES: &str = "/examples";

/// The shipped example at `path`.
pub fn example_at(path: &str) -> Option<&'static ExampleSpec> {
    seeded_examples().find(|spec| spec.vfs_path == path)
}

/// The text of the shipped example at `path`.
pub fn example_text(path: &str) -> Option<&'static str> {
    example_at(path).map(|spec| spec.source)
}

/// The examples' part of the file tree, in the file browser's order: what the tree shows before the
/// reader's store has answered.
pub fn examples_tree() -> FileNode {
    static TREE: OnceLock<FileNode> = OnceLock::new();
    TREE.get_or_init(|| FileNode::from_paths(seeded_examples().map(|spec| spec.vfs_path))).clone()
}

/// `path` as the Studio names a file: one leading slash and no trailing one.
fn normal(path: &str) -> String {
    format!("/{}", path.trim_matches('/'))
}

/// What answers for a path.
enum Layer {
    /// A shipped example, with its text.
    Example(&'static str),
    /// A folder of shipped examples.
    Folder,
    /// A path under `/examples` that no example has.
    Missing,
    /// Anything outside `/examples`: the reader's store.
    Store,
}

fn layer(path: &str) -> Layer {
    let path = normal(path);
    if path != EXAMPLES && !path.starts_with("/examples/") {
        return Layer::Store;
    }
    match example_at(&path) {
        Some(spec) => Layer::Example(spec.source),
        None if example_entries(&path).is_some() => Layer::Folder,
        None => Layer::Missing,
    }
}

/// What the example folder at `dir` holds, or `None` when no example lives under it.
fn example_entries(dir: &str) -> Option<Vec<DirEntry>> {
    let inside = if dir == "/" { "/".to_string() } else { format!("{dir}/") };
    let mut entries: Vec<DirEntry> = Vec::new();
    for spec in seeded_examples() {
        let Some(rest) = spec.vfs_path.strip_prefix(inside.as_str()) else { continue };
        let (name, is_directory) = match rest.split_once('/') {
            Some((folder, _)) => (folder, true),
            None => (rest, false),
        };
        if !entries.iter().any(|entry| entry.name == name) {
            entries.push(DirEntry { name: name.to_string(), is_directory });
        }
    }
    (!entries.is_empty()).then_some(entries)
}

fn read_only(path: &str) -> VfsError {
    VfsError::PermissionDenied(format!("{path}: the shipped examples are read-only. Save a copy under /workspace to change it."))
}

fn not_found(path: &str) -> VfsError {
    VfsError::NotFound(path.to_string())
}

/// The Studio's files: the shipped examples over the reader's store.
#[derive(Clone)]
pub struct StudioFiles<S> {
    store: S,
}

impl<S> StudioFiles<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }

    /// The reader's store underneath.
    pub fn store(&self) -> &S {
        &self.store
    }
}

impl<S: Vfs> StudioFiles<S> {
    /// Remove the copies of the examples an earlier Studio wrote into the reader's store, every file
    /// and folder under `/examples`, and count what went. They have not shown since the examples
    /// came from the site, and nothing else writes there, since the reader's files live in
    /// `/workspace`. A store with no such folder costs one listing. A removal cut short finishes on
    /// the next visit.
    pub async fn retire_old_copies(&self) -> VfsResult<usize> {
        let root = match self.store.list_dir("/").await {
            Ok(entries) => entries,
            Err(VfsError::NotFound(_)) => return Ok(0),
            Err(e) => return Err(e),
        };
        if !root.iter().any(|entry| entry.is_directory && entry.name == EXAMPLES.trim_start_matches('/')) {
            return Ok(0);
        }
        remove_tree(&self.store, EXAMPLES).await
    }
}

/// Remove the folder at `dir` from `store`, its contents first, and count every entry removed.
async fn remove_tree<S: Vfs>(store: &S, dir: &str) -> VfsResult<usize> {
    let mut removed = 0;
    for entry in store.list_dir(dir).await? {
        let path = format!("{dir}/{}", entry.name);
        if entry.is_directory {
            removed += Box::pin(remove_tree(store, &path)).await?;
        } else {
            store.remove(&path).await?;
            removed += 1;
        }
    }
    store.remove(dir).await?;
    Ok(removed + 1)
}

/// What the Studio asks of its files when it opens: the whole file tree (the examples from the
/// site, the reader's own files from the store) and the text of `path`, the file it opened with,
/// or `None` when there is no such file. It writes nothing.
pub async fn open_store<S: Vfs>(files: &StudioFiles<S>, path: &str) -> (VfsResult<FileNode>, Option<String>) {
    let mut tree = FileNode::root();
    let loaded = load_dir_recursive(files, "/", &mut tree).await;
    let read = files.read_to_string(path).await.ok();
    (loaded.map(|()| tree), read)
}

/// Load the folder at `path` from `vfs` into `parent`, every level in the file browser's order.
pub async fn load_dir_recursive<V: Vfs>(vfs: &V, path: &str, parent: &mut FileNode) -> VfsResult<()> {
    let entries = vfs.list_dir(path).await?;

    for entry in entries {
        let full_path = if path == "/" {
            format!("/{}", entry.name)
        } else {
            format!("{}/{}", path, entry.name)
        };

        if entry.is_directory {
            let mut dir_node = FileNode::directory(entry.name.clone(), full_path.clone());
            // Recursively load subdirectories
            let _ = Box::pin(load_dir_recursive(vfs, &full_path, &mut dir_node)).await;
            parent.children.push(dir_node);
        } else {
            parent.children.push(FileNode::file(entry.name, full_path));
        }
    }

    parent.sort_children();

    Ok(())
}

/// A `Vfs` call's answer: a boxed future, `Send` where the trait asks for one.
#[cfg(not(target_arch = "wasm32"))]
type Answer<'a, T> = Pin<Box<dyn Future<Output = VfsResult<T>> + Send + 'a>>;
#[cfg(target_arch = "wasm32")]
type Answer<'a, T> = Pin<Box<dyn Future<Output = VfsResult<T>> + 'a>>;

impl<S: Vfs> Vfs for StudioFiles<S> {
    fn read<'a, 'b, 'f>(&'a self, path: &'b str) -> Answer<'f, Vec<u8>>
    where
        'a: 'f,
        'b: 'f,
        Self: 'f,
    {
        Box::pin(async move {
            match layer(path) {
                Layer::Example(text) => Ok(text.as_bytes().to_vec()),
                Layer::Folder | Layer::Missing => Err(not_found(path)),
                Layer::Store => self.store.read(path).await,
            }
        })
    }

    fn read_to_string<'a, 'b, 'f>(&'a self, path: &'b str) -> Answer<'f, String>
    where
        'a: 'f,
        'b: 'f,
        Self: 'f,
    {
        Box::pin(async move {
            match layer(path) {
                Layer::Example(text) => Ok(text.to_string()),
                Layer::Folder | Layer::Missing => Err(not_found(path)),
                Layer::Store => self.store.read_to_string(path).await,
            }
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
            match layer(path) {
                Layer::Store => self.store.write(path, contents).await,
                _ => Err(read_only(path)),
            }
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
            match layer(path) {
                Layer::Store => self.store.append(path, contents).await,
                _ => Err(read_only(path)),
            }
        })
    }

    fn exists<'a, 'b, 'f>(&'a self, path: &'b str) -> Answer<'f, bool>
    where
        'a: 'f,
        'b: 'f,
        Self: 'f,
    {
        Box::pin(async move {
            match layer(path) {
                Layer::Example(_) | Layer::Folder => Ok(true),
                Layer::Missing => Ok(false),
                Layer::Store => self.store.exists(path).await,
            }
        })
    }

    fn remove<'a, 'b, 'f>(&'a self, path: &'b str) -> Answer<'f, ()>
    where
        'a: 'f,
        'b: 'f,
        Self: 'f,
    {
        Box::pin(async move {
            match layer(path) {
                Layer::Store => self.store.remove(path).await,
                _ => Err(read_only(path)),
            }
        })
    }

    fn create_dir_all<'a, 'b, 'f>(&'a self, path: &'b str) -> Answer<'f, ()>
    where
        'a: 'f,
        'b: 'f,
        Self: 'f,
    {
        Box::pin(async move {
            match layer(path) {
                Layer::Folder => Ok(()),
                Layer::Store => self.store.create_dir_all(path).await,
                Layer::Example(_) | Layer::Missing => Err(read_only(path)),
            }
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
            match (layer(from), layer(to)) {
                (Layer::Store, Layer::Store) => self.store.rename(from, to).await,
                (Layer::Store, _) => Err(read_only(to)),
                _ => Err(read_only(from)),
            }
        })
    }

    fn list_dir<'a, 'b, 'f>(&'a self, path: &'b str) -> Answer<'f, Vec<DirEntry>>
    where
        'a: 'f,
        'b: 'f,
        Self: 'f,
    {
        Box::pin(async move {
            if normal(path) == "/" {
                let mut entries = match self.store.list_dir(path).await {
                    Ok(entries) => entries,
                    Err(VfsError::NotFound(_)) => Vec::new(),
                    Err(e) => return Err(e),
                };
                let examples = EXAMPLES.trim_start_matches('/');
                entries.retain(|entry| entry.name != examples);
                entries.extend(example_entries("/").unwrap_or_default());
                return Ok(entries);
            }
            match layer(path) {
                Layer::Folder => Ok(example_entries(&normal(path)).unwrap_or_default()),
                Layer::Example(_) | Layer::Missing => Err(not_found(path)),
                Layer::Store => self.store.list_dir(path).await,
            }
        })
    }
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/studio_files/tests.rs"]
mod tests;
