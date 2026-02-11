//! LocalStorage persistence layer for user progress.
//!
//! Provides low-level access to browser LocalStorage via WASM bindings.
//! User progress is stored as a JSON string under a single key.
//!
//! # Storage Key
//!
//! All user progress is stored under `logos_user_progress`. This includes:
//! - Exercise completion history
//! - SRS scheduling data
//! - Streak and XP totals
//! - Achievement unlock state
//!
//! # Usage
//!
//! ```no_run
//! use logicaffeine_web::storage;
//!
//! // Load existing progress
//! if let Some(json) = storage::load_raw() {
//!     println!("Loaded: {}", json);
//! }
//!
//! // Save progress
//! storage::save_raw("{}");
//!
//! // Clear all progress (for reset functionality)
//! storage::clear();
//! ```

use wasm_bindgen::prelude::*;

/// LocalStorage key for user progress data.
const PROGRESS_KEY: &str = "logos_user_progress";

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = localStorage, js_name = getItem)]
    fn local_storage_get(key: &str) -> Option<String>;

    #[wasm_bindgen(js_namespace = localStorage, js_name = setItem)]
    fn local_storage_set(key: &str, value: &str);

    #[wasm_bindgen(js_namespace = localStorage, js_name = removeItem)]
    fn local_storage_remove(key: &str);
}

/// Loads the raw JSON progress string from LocalStorage.
///
/// Returns `None` if no progress has been saved or if LocalStorage is unavailable.
pub fn load_raw() -> Option<String> {
    local_storage_get(PROGRESS_KEY)
}

/// Saves a JSON progress string to LocalStorage.
///
/// # Arguments
///
/// * `json` - Serialized progress data. Should be valid JSON produced by
///   `serde_json::to_string()` on a `UserProgress` struct.
pub fn save_raw(json: &str) {
    local_storage_set(PROGRESS_KEY, json);
}

/// Clears all stored progress data.
///
/// Used for "Reset Progress" functionality. This operation is irreversible.
pub fn clear() {
    local_storage_remove(PROGRESS_KEY);
}
