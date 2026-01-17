//! Global application state types.
//!
//! Defines state structures used across the application via Dioxus context.
//! Includes license management, registry authentication, chat state, and
//! Studio mode types.
//!
//! # Key Types
//!
//! | Type | Purpose |
//! |------|---------|
//! | [`LicenseState`] | Subscription validation and plan tiers |
//! | [`RegistryAuthState`] | GitHub OAuth for package registry |
//! | [`AppState`] | Chat history for the REPL interface |
//! | [`StudioMode`] | Active mode in the Studio playground |
//! | [`FileNode`] | Virtual file system tree for Studio |
//! | [`MathDocument`] | Math mode formula collection |
//!
//! # License Tiers
//!
//! | Plan | Commercial Features | Z3 Verification |
//! |------|---------------------|-----------------|
//! | Free | No | No |
//! | Supporter | No | No |
//! | Pro | Yes | Yes |
//! | Premium | Yes | Yes |
//! | Lifetime | Yes | Yes |
//! | Enterprise | Yes | Yes |

use dioxus::prelude::*;
use logicaffeine_language::{compile_with_options, CompileOptions, OutputFormat, Interner, socratic_explanation};

/// API endpoint for license validation.
const LICENSE_VALIDATOR_URL: &str = "https://api.logicaffeine.com/validate";

/// Revalidation interval (24 hours in milliseconds).
const VALIDATION_INTERVAL_MS: f64 = 24.0 * 60.0 * 60.0 * 1000.0;

/// Subscription plan tiers.
///
/// Determines feature access and commercial usage rights.
#[derive(Clone, PartialEq, Debug)]
pub enum LicensePlan {
    /// No active subscription.
    None,
    /// Free tier with basic features.
    Free,
    /// Supporter tier (paid, non-commercial).
    Supporter,
    /// Pro tier with commercial rights and Z3 verification.
    Pro,
    /// Premium tier with all features.
    Premium,
    /// One-time purchase with perpetual access.
    Lifetime,
    /// Organization license with team features.
    Enterprise,
}

impl LicensePlan {
    /// Parses a plan name from a string (case-insensitive).
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "free" => Self::Free,
            "supporter" => Self::Supporter,
            "pro" => Self::Pro,
            "premium" => Self::Premium,
            "lifetime" => Self::Lifetime,
            "enterprise" => Self::Enterprise,
            _ => Self::None,
        }
    }

    /// Returns true if this plan includes commercial usage rights.
    pub fn is_commercial(&self) -> bool {
        matches!(self, Self::Pro | Self::Premium | Self::Lifetime | Self::Enterprise)
    }

    /// Returns true if this is a paid plan (not Free or None).
    pub fn is_paid(&self) -> bool {
        !matches!(self, Self::None | Self::Free)
    }
}

/// License state management with async validation.
///
/// Stored in Dioxus context for app-wide access. Handles license key storage,
/// periodic revalidation, and plan tier checks.
#[derive(Clone, PartialEq)]
pub struct LicenseState {
    /// The license key (Stripe subscription ID).
    pub key: Signal<Option<String>>,
    /// Current subscription plan tier.
    pub plan: Signal<LicensePlan>,
    /// Whether the license has been validated.
    pub is_valid: Signal<bool>,
    /// Timestamp of last successful validation.
    pub validated_at: Signal<Option<f64>>,
    /// True while validation request is in flight.
    pub is_validating: Signal<bool>,
}

impl LicenseState {
    /// Creates a new license state, loading any existing data from LocalStorage.
    pub fn new() -> Self {
        let (key, plan, validated_at) = load_license_from_storage();

        Self {
            key: Signal::new(key),
            plan: Signal::new(plan),
            is_valid: Signal::new(false),
            validated_at: Signal::new(validated_at),
            is_validating: Signal::new(false),
        }
    }

    pub fn has_license(&self) -> bool {
        self.key.read().is_some()
    }

    pub fn is_commercial(&self) -> bool {
        self.plan.read().is_commercial() && *self.is_valid.read()
    }

    pub fn needs_revalidation(&self) -> bool {
        match *self.validated_at.read() {
            Some(timestamp) => {
                let now = js_sys::Date::now();
                now - timestamp > VALIDATION_INTERVAL_MS
            }
            None => true,
        }
    }

    pub fn set_license(&mut self, license_key: String, plan: LicensePlan) {
        self.key.set(Some(license_key.clone()));
        self.plan.set(plan.clone());
        self.is_valid.set(true);
        let now = js_sys::Date::now();
        self.validated_at.set(Some(now));

        save_license_to_storage(&license_key, &plan, now);
    }

    pub fn clear_license(&mut self) {
        self.key.set(None);
        self.plan.set(LicensePlan::None);
        self.is_valid.set(false);
        self.validated_at.set(None);

        clear_license_from_storage();
    }

    pub async fn validate(&mut self) {
        let license_key = match self.key.read().clone() {
            Some(key) => key,
            None => return,
        };

        self.is_validating.set(true);

        match validate_license_async(&license_key).await {
            Ok((is_valid, plan)) => {
                self.is_valid.set(is_valid);
                if is_valid {
                    self.plan.set(plan);
                    let now = js_sys::Date::now();
                    self.validated_at.set(Some(now));
                    save_license_to_storage(&license_key, &self.plan.read(), now);
                }
            }
            Err(_) => {
                self.is_valid.set(false);
            }
        }

        self.is_validating.set(false);
    }
}

async fn validate_license_async(license_key: &str) -> Result<(bool, LicensePlan), String> {
    use gloo_net::http::Request;

    let body = serde_json::json!({ "licenseKey": license_key });

    let response = Request::post(LICENSE_VALIDATOR_URL)
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .map_err(|e| e.to_string())?
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !response.ok() {
        return Ok((false, LicensePlan::None));
    }

    let data: serde_json::Value = response
        .json()
        .await
        .map_err(|e| e.to_string())?;

    let is_valid = data["valid"].as_bool().unwrap_or(false);
    let plan_str = data["plan"].as_str().unwrap_or("none");
    let plan = LicensePlan::from_str(plan_str);

    Ok((is_valid, plan))
}

fn load_license_from_storage() -> (Option<String>, LicensePlan, Option<f64>) {
    if let Some(window) = web_sys::window() {
        if let Ok(Some(storage)) = window.local_storage() {
            let key = storage.get_item("logos_license_key").ok().flatten();
            let plan_str = storage.get_item("logos_license_plan").ok().flatten().unwrap_or_default();
            let validated_at = storage
                .get_item("logos_license_validated_at")
                .ok()
                .flatten()
                .and_then(|s| s.parse::<f64>().ok());

            let plan = LicensePlan::from_str(&plan_str);
            return (key, plan, validated_at);
        }
    }
    (None, LicensePlan::None, None)
}

fn save_license_to_storage(key: &str, plan: &LicensePlan, validated_at: f64) {
    if let Some(window) = web_sys::window() {
        if let Ok(Some(storage)) = window.local_storage() {
            let _ = storage.set_item("logos_license_key", key);
            let plan_str = format!("{:?}", plan).to_lowercase();
            let _ = storage.set_item("logos_license_plan", &plan_str);
            let _ = storage.set_item("logos_license_validated_at", &validated_at.to_string());
        }
    }
}

fn clear_license_from_storage() {
    if let Some(window) = web_sys::window() {
        if let Ok(Some(storage)) = window.local_storage() {
            let _ = storage.remove_item("logos_license_key");
            let _ = storage.remove_item("logos_license_plan");
            let _ = storage.remove_item("logos_license_validated_at");
        }
    }
}

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
        let options = CompileOptions { format: OutputFormat::Unicode };

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
}

impl StudioMode {
    /// Returns the file extension for this mode.
    pub fn extension(&self) -> &'static str {
        match self {
            StudioMode::Logic => "logic",
            StudioMode::Code => "logos",
            StudioMode::Math => "math",
        }
    }

    /// Returns the display name for this mode.
    pub fn display_name(&self) -> &'static str {
        match self {
            StudioMode::Logic => "Logic",
            StudioMode::Code => "Code",
            StudioMode::Math => "Math",
        }
    }

    /// Infer mode from file extension.
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "logic" => Some(StudioMode::Logic),
            "logos" => Some(StudioMode::Code),
            "math" => Some(StudioMode::Math),
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
mod tests {
    use super::*;

    #[test]
    fn test_license_plan_from_str() {
        assert_eq!(LicensePlan::from_str("free"), LicensePlan::Free);
        assert_eq!(LicensePlan::from_str("FREE"), LicensePlan::Free);
        assert_eq!(LicensePlan::from_str("Free"), LicensePlan::Free);
        assert_eq!(LicensePlan::from_str("supporter"), LicensePlan::Supporter);
        assert_eq!(LicensePlan::from_str("pro"), LicensePlan::Pro);
        assert_eq!(LicensePlan::from_str("premium"), LicensePlan::Premium);
        assert_eq!(LicensePlan::from_str("lifetime"), LicensePlan::Lifetime);
        assert_eq!(LicensePlan::from_str("enterprise"), LicensePlan::Enterprise);
        assert_eq!(LicensePlan::from_str("unknown"), LicensePlan::None);
        assert_eq!(LicensePlan::from_str(""), LicensePlan::None);
    }

    #[test]
    fn test_license_plan_is_commercial() {
        assert!(!LicensePlan::None.is_commercial());
        assert!(!LicensePlan::Free.is_commercial());
        assert!(!LicensePlan::Supporter.is_commercial());
        assert!(LicensePlan::Pro.is_commercial());
        assert!(LicensePlan::Premium.is_commercial());
        assert!(LicensePlan::Lifetime.is_commercial());
        assert!(LicensePlan::Enterprise.is_commercial());
    }

    #[test]
    fn test_license_plan_is_paid() {
        assert!(!LicensePlan::None.is_paid());
        assert!(!LicensePlan::Free.is_paid());
        assert!(LicensePlan::Supporter.is_paid());
        assert!(LicensePlan::Pro.is_paid());
        assert!(LicensePlan::Premium.is_paid());
        assert!(LicensePlan::Lifetime.is_paid());
        assert!(LicensePlan::Enterprise.is_paid());
    }
}
