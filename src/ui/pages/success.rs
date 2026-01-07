use dioxus::prelude::*;
use crate::ui::router::Route;
use crate::ui::state::{LicenseState, LicensePlan};
use crate::ui::components::main_nav::{MainNav, ActivePage};

const LICENSE_API_URL: &str = "https://api.logicaffeine.com/session";

const SUCCESS_STYLE: &str = r#"
.success-container {
    min-height: 100vh;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 60px 20px;
    text-align: center;
    max-width: 600px;
    margin: 0 auto;
}

.success-icon {
    width: 80px;
    height: 80px;
    background: linear-gradient(135deg, #00d4ff 0%, #7b2cbf 100%);
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    margin-bottom: 32px;
    font-size: 40px;
}

.success-title {
    font-size: 36px;
    font-weight: 700;
    color: #fff;
    margin-bottom: 16px;
}

.success-message {
    color: #aaa;
    font-size: 18px;
    line-height: 1.6;
    margin-bottom: 32px;
}

.license-box {
    background: rgba(0, 212, 255, 0.1);
    border: 1px solid rgba(0, 212, 255, 0.3);
    border-radius: 12px;
    padding: 24px;
    margin-bottom: 32px;
    width: 100%;
    max-width: 400px;
}

.license-label {
    color: #00d4ff;
    font-size: 14px;
    font-weight: 600;
    margin-bottom: 12px;
    text-transform: uppercase;
    letter-spacing: 1px;
}

.license-key {
    background: rgba(0, 0, 0, 0.3);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 8px;
    padding: 16px;
    font-family: monospace;
    font-size: 14px;
    color: #fff;
    word-break: break-all;
    margin-bottom: 12px;
}

.copy-btn {
    background: linear-gradient(135deg, #00d4ff 0%, #7b2cbf 100%);
    color: white;
    border: none;
    padding: 10px 20px;
    border-radius: 8px;
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.2s ease;
}

.copy-btn:hover {
    transform: translateY(-1px);
    box-shadow: 0 4px 12px rgba(0, 212, 255, 0.3);
}

.license-saved {
    color: #4ade80;
    font-size: 13px;
    margin-top: 12px;
}

.success-actions {
    display: flex;
    flex-direction: column;
    gap: 16px;
    width: 100%;
    max-width: 320px;
}

.btn-primary {
    display: block;
    background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
    color: white;
    padding: 16px 32px;
    border-radius: 12px;
    font-size: 16px;
    font-weight: 600;
    text-decoration: none;
    text-align: center;
    transition: all 0.2s ease;
}

.btn-primary:hover {
    transform: translateY(-2px);
    box-shadow: 0 6px 20px rgba(102, 126, 234, 0.4);
}

.btn-secondary {
    display: block;
    background: transparent;
    color: #667eea;
    padding: 16px 32px;
    border: 1px solid #667eea;
    border-radius: 12px;
    font-size: 16px;
    font-weight: 500;
    text-decoration: none;
    text-align: center;
    transition: all 0.2s ease;
}

.btn-secondary:hover {
    background: rgba(102, 126, 234, 0.1);
}

.success-note {
    margin-top: 40px;
    padding: 20px;
    background: rgba(255, 255, 255, 0.05);
    border-radius: 12px;
    border: 1px solid rgba(255, 255, 255, 0.1);
}

.success-note p {
    color: #888;
    font-size: 14px;
    margin: 0;
}

.success-note a {
    color: #00d4ff;
    text-decoration: none;
}

.success-note a:hover {
    text-decoration: underline;
}

.loading-spinner {
    width: 40px;
    height: 40px;
    border: 3px solid rgba(0, 212, 255, 0.3);
    border-top: 3px solid #00d4ff;
    border-radius: 50%;
    animation: spin 1s linear infinite;
    margin: 20px auto;
}

@keyframes spin {
    0% { transform: rotate(0deg); }
    100% { transform: rotate(360deg); }
}

.error-message {
    color: #ef4444;
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: 8px;
    padding: 16px;
    margin-bottom: 24px;
}
"#;

const STRIPE_CUSTOMER_PORTAL: &str = "https://billing.stripe.com/p/login/8x200l3VN98D7qa1SMe3e00";

fn get_session_id_from_url() -> Option<String> {
    let window = web_sys::window()?;
    let location = window.location();
    let search = location.search().ok()?;

    let params = web_sys::UrlSearchParams::new_with_str(&search).ok()?;
    params.get("session_id")
}

fn save_license_to_storage(license_key: &str, plan: &str) {
    if let Some(window) = web_sys::window() {
        if let Ok(Some(storage)) = window.local_storage() {
            let _ = storage.set_item("logos_license_key", license_key);
            let _ = storage.set_item("logos_license_plan", plan);
            let timestamp = js_sys::Date::now().to_string();
            let _ = storage.set_item("logos_license_validated_at", &timestamp);
        }
    }
}

fn copy_to_clipboard(text: &str) {
    if let Some(window) = web_sys::window() {
        let clipboard = window.navigator().clipboard();
        let _ = clipboard.write_text(text);
    }
}

#[derive(Clone, PartialEq)]
enum LicenseStatus {
    Loading,
    Success { subscription_id: String, plan: String },
    Error(String),
    NoSession,
}

async fn fetch_license_from_session(session_id: String) -> LicenseStatus {
    use gloo_net::http::Request;

    let body = serde_json::json!({ "sessionId": session_id });

    let response = Request::post(LICENSE_API_URL)
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .unwrap()
        .send()
        .await;

    match response {
        Ok(resp) => {
            if resp.ok() {
                match resp.json::<serde_json::Value>().await {
                    Ok(data) => {
                        let subscription_id = data["subscriptionId"]
                            .as_str()
                            .unwrap_or("")
                            .to_string();
                        let plan = data["plan"]
                            .as_str()
                            .unwrap_or("unknown")
                            .to_string();
                        LicenseStatus::Success { subscription_id, plan }
                    }
                    Err(_) => LicenseStatus::Error("Failed to parse response".to_string()),
                }
            } else {
                LicenseStatus::Error("License lookup failed".to_string())
            }
        }
        Err(e) => LicenseStatus::Error(format!("Network error: {}", e)),
    }
}

#[component]
pub fn Success() -> Element {
    let mut license_status = use_signal(|| LicenseStatus::Loading);
    let mut copied = use_signal(|| false);
    let mut saved = use_signal(|| false);
    let license_state = use_context::<LicenseState>();

    use_effect(move || {
        let mut license_state = license_state.clone();
        spawn(async move {
            if let Some(session_id) = get_session_id_from_url() {
                let result = fetch_license_from_session(session_id).await;
                if let LicenseStatus::Success { ref subscription_id, ref plan } = result {
                    save_license_to_storage(subscription_id, plan);
                    license_state.set_license(
                        subscription_id.clone(),
                        LicensePlan::from_str(plan),
                    );
                    saved.set(true);
                }
                license_status.set(result);
            } else {
                license_status.set(LicenseStatus::NoSession);
            }
        });
    });

    let on_copy = move |_| {
        if let LicenseStatus::Success { ref subscription_id, .. } = *license_status.read() {
            copy_to_clipboard(subscription_id);
            copied.set(true);
        }
    };

    let (has_license, license_key) = match &*license_status.read() {
        LicenseStatus::Success { subscription_id, .. } => (true, subscription_id.clone()),
        _ => (false, String::new()),
    };

    let is_loading = matches!(*license_status.read(), LicenseStatus::Loading);

    rsx! {
        style { "{SUCCESS_STYLE}" }

        MainNav { active: ActivePage::Pricing, subtitle: Some("Payment Complete"), show_nav_links: false }

        div { class: "success-container",
            div { class: "success-icon", "✓" }

            h1 { class: "success-title", "Thank You!" }

            p { class: "success-message",
                "Your payment was successful. Welcome to logicaffeine! "
                "You now have access to use LOGOS for commercial purposes."
            }

            if is_loading {
                div { class: "loading-spinner" }
                p { class: "success-message", "Retrieving your license..." }
            }

            match &*license_status.read() {
                LicenseStatus::Error(msg) => rsx! {
                    div { class: "error-message", "{msg}" }
                },
                LicenseStatus::NoSession => rsx! {
                    div { class: "error-message", "No checkout session found. Please try again." }
                },
                _ => rsx! {}
            }

            if has_license {
                div { class: "license-box",
                    div { class: "license-label", "Your License Key" }
                    div { class: "license-key", "{license_key}" }
                    button {
                        class: "copy-btn",
                        onclick: on_copy,
                        if *copied.read() { "Copied!" } else { "Copy to Clipboard" }
                    }
                    if *saved.read() {
                        div { class: "license-saved",
                            "✓ License saved to your browser"
                        }
                    }
                }
            }

            div { class: "success-actions",
                Link {
                    class: "btn-primary",
                    to: Route::Studio {},
                    "Open Studio"
                }

                a {
                    class: "btn-secondary",
                    href: STRIPE_CUSTOMER_PORTAL,
                    target: "_blank",
                    "Manage Subscription"
                }
            }

            div { class: "success-note",
                p {
                    "Save your license key somewhere safe. "
                    "Need help? Contact us at "
                    a { href: "mailto:tristen@brahmastra-labs.com", "tristen@brahmastra-labs.com" }
                    "."
                }
            }
        }
    }
}
