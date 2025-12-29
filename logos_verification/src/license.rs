//! License validation for LOGOS verification.
//!
//! Uses the existing Stripe-based license system. License keys are
//! Stripe subscription IDs (`sub_*` format).

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{VerificationError, VerificationResult};

/// The license validation API endpoint.
const LICENSE_API: &str = "https://api.logicaffeine.com/validate";

/// Cache duration in seconds (24 hours).
const CACHE_DURATION_SECS: u64 = 24 * 60 * 60;

/// License plan tiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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
    /// Check if this plan allows verification.
    pub fn can_verify(&self) -> bool {
        matches!(
            self,
            Self::Pro | Self::Premium | Self::Lifetime | Self::Enterprise
        )
    }

    /// Parse a plan from a string.
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
}

impl std::fmt::Display for LicensePlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "None"),
            Self::Free => write!(f, "Free"),
            Self::Supporter => write!(f, "Supporter"),
            Self::Pro => write!(f, "Pro"),
            Self::Premium => write!(f, "Premium"),
            Self::Lifetime => write!(f, "Lifetime"),
            Self::Enterprise => write!(f, "Enterprise"),
        }
    }
}

/// Cached license validation result.
#[derive(Debug, Serialize, Deserialize)]
struct CachedLicense {
    key: String,
    plan: String,
    valid: bool,
    validated_at: u64,
}

/// Response from the license validation API.
#[derive(Debug, Deserialize)]
struct LicenseResponse {
    valid: bool,
    #[serde(default)]
    plan: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

/// License validator that checks keys against the API with caching.
pub struct LicenseValidator {
    cache_path: PathBuf,
}

impl LicenseValidator {
    /// Create a new license validator.
    pub fn new() -> Self {
        let cache_dir = dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("logos");

        // Ensure cache directory exists
        let _ = fs::create_dir_all(&cache_dir);

        Self {
            cache_path: cache_dir.join("verification_license.json"),
        }
    }

    /// Validate a license key.
    ///
    /// Returns the plan if valid, or an error if invalid or network fails.
    pub fn validate(&self, key: &str) -> VerificationResult<LicensePlan> {
        // Check key format
        if !key.starts_with("sub_") {
            return Err(VerificationError::license_invalid(
                "Invalid license key format. Keys should start with 'sub_'.",
            ));
        }

        // Check cache first
        if let Some(cached) = self.load_cache() {
            if cached.key == key && self.is_cache_fresh(&cached) {
                let plan = LicensePlan::from_str(&cached.plan);
                if cached.valid && plan.can_verify() {
                    return Ok(plan);
                } else if !cached.valid {
                    return Err(VerificationError::license_invalid("License key is invalid"));
                } else {
                    return Err(VerificationError::insufficient_plan(plan.to_string()));
                }
            }
        }

        // Validate with API
        match self.validate_with_api(key) {
            Ok((valid, plan)) => {
                // Cache the result
                self.save_cache(key, &plan.to_string().to_lowercase(), valid);

                if valid && plan.can_verify() {
                    Ok(plan)
                } else if !valid {
                    Err(VerificationError::license_invalid("License key is invalid"))
                } else {
                    Err(VerificationError::insufficient_plan(plan.to_string()))
                }
            }
            Err(e) => {
                // If network fails, try to use stale cache
                if let Some(cached) = self.load_cache() {
                    if cached.key == key {
                        eprintln!(
                            "Warning: Could not validate license ({}). Using cached result.",
                            e
                        );
                        let plan = LicensePlan::from_str(&cached.plan);
                        if cached.valid && plan.can_verify() {
                            return Ok(plan);
                        }
                    }
                }
                Err(VerificationError::license_invalid(format!(
                    "Could not validate license: {}",
                    e
                )))
            }
        }
    }

    /// Validate the key against the API.
    fn validate_with_api(&self, key: &str) -> Result<(bool, LicensePlan), String> {
        let response = ureq::post(LICENSE_API)
            .set("Content-Type", "application/json")
            .send_json(ureq::json!({ "licenseKey": key }))
            .map_err(|e| format!("Network error: {}", e))?;

        let body: LicenseResponse = response
            .into_json()
            .map_err(|e| format!("Invalid response: {}", e))?;

        if let Some(error) = body.error {
            return Err(error);
        }

        let plan = body
            .plan
            .map(|p| LicensePlan::from_str(&p))
            .unwrap_or(LicensePlan::None);

        Ok((body.valid, plan))
    }

    /// Load cached license validation.
    fn load_cache(&self) -> Option<CachedLicense> {
        let content = fs::read_to_string(&self.cache_path).ok()?;
        serde_json::from_str(&content).ok()
    }

    /// Save license validation to cache.
    fn save_cache(&self, key: &str, plan: &str, valid: bool) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let cached = CachedLicense {
            key: key.to_string(),
            plan: plan.to_string(),
            valid,
            validated_at: now,
        };

        if let Ok(json) = serde_json::to_string_pretty(&cached) {
            let _ = fs::write(&self.cache_path, json);
        }
    }

    /// Check if the cache is still fresh (< 24 hours).
    fn is_cache_fresh(&self, cached: &CachedLicense) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        now.saturating_sub(cached.validated_at) < CACHE_DURATION_SECS
    }
}

impl Default for LicenseValidator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plan_can_verify() {
        assert!(!LicensePlan::None.can_verify());
        assert!(!LicensePlan::Free.can_verify());
        assert!(!LicensePlan::Supporter.can_verify());
        assert!(LicensePlan::Pro.can_verify());
        assert!(LicensePlan::Premium.can_verify());
        assert!(LicensePlan::Lifetime.can_verify());
        assert!(LicensePlan::Enterprise.can_verify());
    }

    #[test]
    fn test_invalid_key_format() {
        let validator = LicenseValidator::new();
        let result = validator.validate("invalid_key");
        assert!(result.is_err());
    }
}
