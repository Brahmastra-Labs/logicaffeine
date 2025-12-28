use dioxus::prelude::*;
use crate::{compile_with_options, CompileOptions, OutputFormat};

const LICENSE_VALIDATOR_URL: &str = "https://api.logicaffeine.com/validate";
const VALIDATION_INTERVAL_MS: f64 = 24.0 * 60.0 * 60.0 * 1000.0; // 24 hours

#[derive(Clone, PartialEq, Debug)]
pub enum LicensePlan {
    None,
    Free,
    Supporter,
    Pro,
    Premium,
    Lifetime,
    Enterprise,
}

impl LicensePlan {
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

    pub fn is_commercial(&self) -> bool {
        matches!(self, Self::Pro | Self::Premium | Self::Lifetime | Self::Enterprise)
    }

    pub fn is_paid(&self) -> bool {
        !matches!(self, Self::None | Self::Free)
    }
}

#[derive(Clone, PartialEq)]
pub struct LicenseState {
    pub key: Signal<Option<String>>,
    pub plan: Signal<LicensePlan>,
    pub is_valid: Signal<bool>,
    pub validated_at: Signal<Option<f64>>,
    pub is_validating: Signal<bool>,
}

impl LicenseState {
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

#[derive(Clone, PartialEq)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

#[derive(Clone, PartialEq)]
pub enum Role {
    User,
    System,
    Error,
}

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
                let interner = crate::Interner::new();
                let advice = crate::socratic_explanation(&e, &interner);
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
// Phase 39: GitHub Auth State for Package Registry
// ============================================================

const REGISTRY_API_URL: &str = "https://registry.logicaffeine.com";

#[derive(Clone, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub struct GitHubUser {
    pub id: String,
    pub login: String,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Clone, PartialEq)]
pub struct RegistryAuthState {
    pub user: Signal<Option<GitHubUser>>,
    pub token: Signal<Option<String>>,
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

// Package types for registry
#[derive(Clone, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub struct RegistryPackage {
    pub name: String,
    pub description: Option<String>,
    pub latest_version: Option<String>,
    pub owner: String,
    pub owner_avatar: Option<String>,
    pub verified: bool,
    pub downloads: u64,
    pub keywords: Vec<String>,
}

#[derive(Clone, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub struct PackageVersion {
    pub version: String,
    pub published_at: String,
    pub size: u64,
    pub yanked: bool,
}

#[derive(Clone, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub struct PackageDetails {
    pub name: String,
    pub description: Option<String>,
    pub owner: String,
    pub owner_avatar: Option<String>,
    pub repository: Option<String>,
    pub homepage: Option<String>,
    pub license: Option<String>,
    pub keywords: Vec<String>,
    pub verified: bool,
    pub downloads: u64,
    pub readme: Option<String>,
    pub versions: Vec<PackageVersion>,
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
