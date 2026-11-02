//! Global application state types.
//!
//! Defines state structures used across the application via Dioxus context.
//! Includes registry authentication, chat state, and
//! Studio mode types.
//!
//! # Key Types
//!
//! | Type | Purpose |
//! |------|---------|
//! | [`RegistryAuthState`] | GitHub OAuth for package registry |
//! | [`AppState`] | Chat history for the REPL interface |
//! | [`StudioMode`] | Active mode in the Studio playground |
//! | [`FileNode`] | Virtual file system tree for Studio |
//! | [`MathDocument`] | Math mode formula collection |
//!

use dioxus::prelude::*;
use logicaffeine_language::{
    compile_with_options, socratic_explanation, CompileOptions, Interner, OutputFormat,
};

/// A message in the REPL chat history.
#[derive(Clone, PartialEq)]
pub struct ChatMessage {
    /// The sender of the message.
    pub role: Role,
    /// The message text (input or compiled output).
    pub content: String,
}

/// Message sender role in the chat interface.
#[derive(Clone, PartialEq)]
pub enum Role {
    /// User-entered input.
    User,
    /// Successful compilation result.
    System,
    /// Compilation error with Socratic guidance.
    Error,
}

/// REPL chat state for the Landing page.
///
/// Maintains a history of user inputs and system responses.
#[derive(Clone, Copy)]
pub struct AppState {
    history: Signal<Vec<ChatMessage>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            history: Signal::new(vec![ChatMessage {
                role: Role::System,
                content: "The Council is assembled. State your premise.".to_string(),
            }]),
        }
    }

    pub fn add_user_message(&mut self, text: String) {
        self.history.write().push(ChatMessage {
            role: Role::User,
            content: text.clone(),
        });
        self.process_logic(text);
    }

    fn process_logic(&mut self, input: String) {
        let options = CompileOptions {
            format: OutputFormat::Unicode,
            pragmatic: false,
        };

        let response = match compile_with_options(&input, options) {
            Ok(logic) => ChatMessage {
                role: Role::System,
                content: logic,
            },
            Err(e) => {
                let interner = Interner::new();
                let advice = socratic_explanation(&e, &interner);
                ChatMessage {
                    role: Role::Error,
                    content: advice,
                }
            }
        };
        self.history.write().push(response);
    }

    pub fn get_history(&self) -> Vec<ChatMessage> {
        self.history.read().clone()
    }
}

// ============================================================
// Package Registry Authentication
// ============================================================

/// Base URL for the package registry API.
const REGISTRY_API_URL: &str = "https://registry.logicaffeine.com";

/// GitHub user profile data from OAuth.
#[derive(Clone, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub struct GitHubUser {
    /// GitHub user ID (numeric string).
    pub id: String,
    /// GitHub username.
    pub login: String,
    /// Display name (may be empty).
    pub name: Option<String>,
    /// Profile picture URL.
    pub avatar_url: Option<String>,
}

/// Authentication state for the package registry.
///
/// Manages GitHub OAuth tokens for publishing and starring packages.
#[derive(Clone, PartialEq)]
pub struct RegistryAuthState {
    /// Currently authenticated GitHub user.
    pub user: Signal<Option<GitHubUser>>,
    /// OAuth access token for API calls.
    pub token: Signal<Option<String>>,
    /// True while authentication is in progress.
    pub is_loading: Signal<bool>,
}

impl RegistryAuthState {
    pub fn new() -> Self {
        let (token, user) = load_registry_auth_from_storage();
        Self {
            user: Signal::new(user),
            token: Signal::new(token),
            is_loading: Signal::new(false),
        }
    }

    pub fn is_authenticated(&self) -> bool {
        self.token.read().is_some()
    }

    pub fn login(&mut self, token: String, user: GitHubUser) {
        self.token.set(Some(token.clone()));
        self.user.set(Some(user.clone()));
        save_registry_auth_to_storage(&token, &user);
    }

    pub fn logout(&mut self) {
        self.token.set(None);
        self.user.set(None);
        clear_registry_auth_from_storage();
    }

    pub fn get_auth_url() -> String {
        format!("{}/auth/github", REGISTRY_API_URL)
    }
}

fn load_registry_auth_from_storage() -> (Option<String>, Option<GitHubUser>) {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(window) = web_sys::window() {
            if let Ok(Some(storage)) = window.local_storage() {
                let token = storage.get_item("logos_registry_token").ok().flatten();
                let user_json = storage.get_item("logos_registry_user").ok().flatten();
                let user = user_json.and_then(|j| serde_json::from_str(&j).ok());
                return (token, user);
            }
        }
    }
    (None, None)
}

fn save_registry_auth_to_storage(token: &str, user: &GitHubUser) {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(window) = web_sys::window() {
            if let Ok(Some(storage)) = window.local_storage() {
                let _ = storage.set_item("logos_registry_token", token);
                if let Ok(json) = serde_json::to_string(user) {
                    let _ = storage.set_item("logos_registry_user", &json);
                }
            }
        }
    }
}

fn clear_registry_auth_from_storage() {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(window) = web_sys::window() {
            if let Ok(Some(storage)) = window.local_storage() {
                let _ = storage.remove_item("logos_registry_token");
                let _ = storage.remove_item("logos_registry_user");
            }
        }
    }
}

// ============================================================
// Package Registry Types
// ============================================================

/// Summary information for a package listing.
#[derive(Clone, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub struct RegistryPackage {
    /// Package name (unique identifier).
    pub name: String,
    /// Short description for search results.
    pub description: Option<String>,
    /// Most recent version number.
    pub latest_version: Option<String>,
    /// GitHub username of the package owner.
    pub owner: String,
    /// Owner's GitHub avatar URL.
    pub owner_avatar: Option<String>,
    /// Whether the package is officially verified.
    pub verified: bool,
    /// Total download count.
    pub downloads: u64,
    /// Searchable tags.
    pub keywords: Vec<String>,
}

/// Metadata for a specific package version.
#[derive(Clone, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub struct PackageVersion {
    /// Semantic version string (e.g., "1.0.0").
    pub version: String,
    /// ISO 8601 publication timestamp.
    pub published_at: String,
    /// Package size in bytes.
    pub size: u64,
    /// Whether this version has been yanked.
    pub yanked: bool,
}

/// Full package detail for the package page.
#[derive(Clone, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub struct PackageDetails {
    /// Package name.
    pub name: String,
    /// Full description.
    pub description: Option<String>,
    /// Package owner's GitHub username.
    pub owner: String,
    /// Owner's avatar URL.
    pub owner_avatar: Option<String>,
    /// Repository URL.
    pub repository: Option<String>,
    /// Documentation or homepage URL.
    pub homepage: Option<String>,
    /// SPDX license identifier.
    pub license: Option<String>,
    /// Searchable keywords.
    pub keywords: Vec<String>,
    /// Official verification status.
    pub verified: bool,
    /// Total downloads across all versions.
    pub downloads: u64,
    /// README content (markdown).
    pub readme: Option<String>,
    /// All published versions.
    pub versions: Vec<PackageVersion>,
}

// ============================================================
// Studio Mode State
// ============================================================

/// The active mode in the Studio playground.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum StudioMode {
    /// English to First-Order Logic translation (default)
    #[default]
    Logic,
    /// Vernacular REPL for proof development and .logos files
    Code,
    /// Visual formula builder with LaTeX preview
    Math,
    /// English hardware specs to SystemVerilog Assertions with in-browser,
    /// kernel-certified proving (no Z3 in the bundle).
    Hardware,
    /// A LOGOS page (`## Page`) compiled to wasm in-browser and rendered LIVE in a
    /// sandboxed iframe — edit the page's source and see it run.
    Web,
}

impl StudioMode {
    /// Every mode, in display order.
    pub const ALL: [StudioMode; 5] = [
        StudioMode::Logic,
        StudioMode::Code,
        StudioMode::Math,
        StudioMode::Hardware,
        StudioMode::Web,
    ];

    /// The mode a Studio file opens in. The Studio has no mode switch — opening a file is
    /// choosing its mode — so this is the one place a path becomes a mode.
    ///
    /// The extension names the mode, except for `.logos`: it is the source language of
    /// Code, Math and Web alike, so a `math/` or `web/` folder decides which. `None` is a
    /// file with no LOGOS extension; the Studio keeps its current mode for it.
    pub fn for_path(path: &str) -> Option<Self> {
        let (folders, file) = path.rsplit_once('/').unwrap_or(("", path));
        let (stem, ext) = file.rsplit_once('.')?;
        if stem.is_empty() {
            return None;
        }
        let ext = ext.to_ascii_lowercase();
        let in_folder = |name: &str| folders.split('/').any(|f| f == name);
        match ext.as_str() {
            "logos" if in_folder("math") => Some(StudioMode::Math),
            "logos" if in_folder("web") => Some(StudioMode::Web),
            "vernac" => Some(StudioMode::Math),
            "v" => Some(StudioMode::Hardware),
            other => Self::from_extension(other),
        }
    }

    /// Returns the file extension for this mode.
    pub fn extension(&self) -> &'static str {
        match self {
            StudioMode::Logic => "logic",
            StudioMode::Code => "logos",
            StudioMode::Math => "math",
            StudioMode::Hardware => "hw",
            StudioMode::Web => "web",
        }
    }

    /// Returns the display name for this mode.
    pub fn display_name(&self) -> &'static str {
        match self {
            StudioMode::Logic => "Logic",
            StudioMode::Code => "Code",
            StudioMode::Math => "Math",
            StudioMode::Hardware => "Hardware",
            StudioMode::Web => "Web",
        }
    }

    /// The mode a `.logos` source opens in: a file that declares a `## Page` is a web page and
    /// opens in the Web tab, where it runs live; any other program opens in Code. The header is read
    /// by the real lexer, so a page is recognised exactly when the compiler would mount one.
    pub fn for_logos_source(source: &str) -> Self {
        use logicaffeine_language::{BlockType, TokenType};
        let mut interner = Interner::new();
        let declares_page = logicaffeine_language::Lexer::new(source, &mut interner)
            .tokenize()
            .iter()
            .any(|t| {
                matches!(
                    t.kind,
                    TokenType::BlockHeader {
                        block_type: BlockType::Page { .. }
                    }
                )
            });
        if declares_page {
            StudioMode::Web
        } else {
            StudioMode::Code
        }
    }

    /// The mode a file opens in once its text is known — the one rule the file browser and the
    /// Studio's opening share. The path decides ([`Self::for_path`]); a `.logos` file the path
    /// leaves to Code opens in Web when it declares a page ([`Self::for_logos_source`]). `None` is
    /// a file with no LOGOS extension.
    pub fn for_file(path: &str, text: &str) -> Option<Self> {
        match Self::for_path(path)? {
            StudioMode::Code => Some(Self::for_logos_source(text)),
            mode => Some(mode),
        }
    }

    /// Infer mode from file extension.
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "logic" => Some(StudioMode::Logic),
            "logos" => Some(StudioMode::Code),
            "math" => Some(StudioMode::Math),
            "hw" => Some(StudioMode::Hardware),
            "web" => Some(StudioMode::Web),
            _ => None,
        }
    }
}

/// A node in the file tree representing a file or directory.
#[derive(Clone, PartialEq, Debug)]
pub struct FileNode {
    /// Name of the file or directory (not full path).
    pub name: String,
    /// Full path from VFS root.
    pub path: String,
    /// True if this is a directory.
    pub is_directory: bool,
    /// Child nodes (empty for files).
    pub children: Vec<FileNode>,
    /// Whether this directory is expanded in the UI.
    pub expanded: bool,
}

impl FileNode {
    /// Create a new file node.
    pub fn file(name: String, path: String) -> Self {
        Self {
            name,
            path,
            is_directory: false,
            children: Vec::new(),
            expanded: false,
        }
    }

    /// Create a new directory node.
    pub fn directory(name: String, path: String) -> Self {
        Self {
            name,
            path,
            is_directory: true,
            children: Vec::new(),
            expanded: true,
        }
    }

    /// Create the root node for the file tree.
    pub fn root() -> Self {
        Self {
            name: "workspace".to_string(),
            path: "/".to_string(),
            is_directory: true,
            children: Vec::new(),
            expanded: true,
        }
    }

    /// The tree of the files at `paths` (VFS paths), each under its folders and every level in the
    /// file browser's order: the tree the Studio loads from a file system that holds exactly these
    /// files.
    pub fn from_paths<'a>(paths: impl IntoIterator<Item = &'a str>) -> Self {
        let mut root = Self::root();
        for path in paths {
            let mut node = &mut root;
            let mut at = String::new();
            let mut names = path.trim_start_matches('/').split('/').peekable();
            while let Some(name) = names.next() {
                at = format!("{at}/{name}");
                let is_directory = names.peek().is_some();
                let i = match node
                    .children
                    .iter()
                    .position(|c| c.name == name && c.is_directory == is_directory)
                {
                    Some(i) => i,
                    None => {
                        let child = if is_directory {
                            Self::directory(name.to_string(), at.clone())
                        } else {
                            Self::file(name.to_string(), at.clone())
                        };
                        node.children.push(child);
                        node.children.len() - 1
                    }
                };
                node = &mut node.children[i];
            }
        }
        fn sort_all(node: &mut FileNode) {
            node.sort_children();
            node.children.iter_mut().for_each(sort_all);
        }
        sort_all(&mut root);
        root
    }

    /// Put this directory's entries in the order the file browser shows them: directories first;
    /// the Studio's code categories in learning order (basics first, projects last), everything
    /// else alphabetically. Unknown directory names rank after the curated set, so a future
    /// category still appears without touching this list.
    pub fn sort_children(&mut self) {
        fn category_rank(name: &str) -> usize {
            const LEARNING_ORDER: [&str; 15] = [
                "basics",
                "strings",
                "collections",
                "functions",
                "types",
                "algorithms",
                "memory",
                "time-and-money",
                "concurrency",
                "distributed",
                "networking",
                "security",
                "crypto",
                "verification",
                "projects",
            ];
            LEARNING_ORDER
                .iter()
                .position(|c| *c == name)
                .unwrap_or(LEARNING_ORDER.len())
        }
        self.children
            .sort_by(|a, b| match (a.is_directory, b.is_directory) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => category_rank(&a.name)
                    .cmp(&category_rank(&b.name))
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
            });
    }

    /// This tree with each folder open or closed as `shown` has it, wherever `shown` has the same
    /// folder: a tree loaded again keeps the reader's choices, and a folder new to it starts open.
    pub fn with_expansion_of(mut self, shown: &FileNode) -> Self {
        fn collect(node: &FileNode, open: &mut std::collections::HashMap<String, bool>) {
            if node.is_directory {
                open.insert(node.path.clone(), node.expanded);
                node.children.iter().for_each(|child| collect(child, open));
            }
        }
        fn apply(node: &mut FileNode, open: &std::collections::HashMap<String, bool>) {
            if node.is_directory {
                if let Some(&expanded) = open.get(&node.path) {
                    node.expanded = expanded;
                }
                node.children
                    .iter_mut()
                    .for_each(|child| apply(child, open));
            }
        }
        let mut open = std::collections::HashMap::new();
        collect(shown, &mut open);
        apply(&mut self, &open);
        self
    }

    /// The first file under the directory at `dir`, in the order the tree shows it.
    pub fn first_file_under(&self, dir: &str) -> Option<&str> {
        let inside = |path: &str| {
            path.strip_prefix(dir)
                .is_some_and(|rest| rest.starts_with('/'))
        };
        if !self.is_directory {
            return inside(&self.path).then_some(self.path.as_str());
        }
        self.children
            .iter()
            .find_map(|child| child.first_file_under(dir))
    }

    /// Toggle the expanded state of a directory.
    pub fn toggle_expanded(&mut self) {
        if self.is_directory {
            self.expanded = !self.expanded;
        }
    }

    /// Find a node by path (mutable).
    pub fn find_mut(&mut self, path: &str) -> Option<&mut FileNode> {
        if self.path == path {
            return Some(self);
        }
        for child in &mut self.children {
            if let Some(found) = child.find_mut(path) {
                return Some(found);
            }
        }
        None
    }
}

/// A line in the REPL output history.
#[derive(Clone, PartialEq, Debug)]
pub struct ReplLine {
    /// The input command.
    pub input: String,
    /// The output (result or error).
    pub output: Result<String, String>,
    /// Whether this was executed successfully.
    pub success: bool,
}

impl ReplLine {
    pub fn success(input: String, output: String) -> Self {
        Self {
            input,
            output: Ok(output),
            success: true,
        }
    }

    pub fn error(input: String, error: String) -> Self {
        Self {
            input,
            output: Err(error),
            success: false,
        }
    }
}

/// A formula entry for Math mode.
#[derive(Clone, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub struct MathFormula {
    /// The LaTeX source.
    pub latex: String,
    /// Optional label/name for the formula.
    pub label: Option<String>,
}

/// Math mode file format.
#[derive(Clone, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub struct MathDocument {
    /// Document name.
    pub name: String,
    /// List of formulas.
    pub formulas: Vec<MathFormula>,
    /// Index of the currently active formula.
    #[serde(default)]
    pub active_index: usize,
}

impl Default for MathDocument {
    fn default() -> Self {
        Self {
            name: "Untitled".to_string(),
            formulas: vec![MathFormula {
                latex: String::new(),
                label: None,
            }],
            active_index: 0,
        }
    }
}

#[cfg(test)]
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/state/first_file_tests.rs"]
mod first_file_tests;

#[cfg(test)]
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/state/tree_order_tests.rs"]
mod tree_order_tests;

#[cfg(test)]
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/state/file_mode_tests.rs"]
mod file_mode_tests;

#[cfg(test)]
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/state/tests.rs"]
mod tests;
