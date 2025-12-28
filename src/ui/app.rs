use dioxus::prelude::*;
use crate::ui::router::Route;
use crate::ui::state::{LicenseState, RegistryAuthState};

const GLOBAL_STYLE: &str = r#"
* {
    margin: 0;
    padding: 0;
    box-sizing: border-box;
}

html, body {
    height: 100%;
    background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f3460 100%);
    color: #e8e8e8;
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
    overflow-x: hidden;
}

#main {
    min-height: 100vh;
}

a {
    color: inherit;
    text-decoration: none;
}

.chat-area {
    flex: 1;
    overflow-y: auto;
    padding: 30px;
    display: flex;
    flex-direction: column;
    gap: 16px;
}

.message {
    max-width: 75%;
    padding: 14px 20px;
    border-radius: 16px;
    line-height: 1.6;
    animation: fadeIn 0.3s ease;
}

@keyframes fadeIn {
    from { opacity: 0; transform: translateY(10px); }
    to { opacity: 1; transform: translateY(0); }
}

.message.user {
    align-self: flex-end;
    background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
    color: white;
    border-bottom-right-radius: 4px;
    box-shadow: 0 4px 15px rgba(102, 126, 234, 0.3);
}

.message.system {
    align-self: flex-start;
    background: rgba(255, 255, 255, 0.08);
    border: 1px solid rgba(255, 255, 255, 0.15);
    border-bottom-left-radius: 4px;
    font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;
    font-size: 18px;
    color: #00d4ff;
    text-shadow: 0 0 20px rgba(0, 212, 255, 0.3);
}

.message.error {
    align-self: flex-start;
    background: linear-gradient(135deg, #ff6b6b 0%, #c92a2a 100%);
    color: white;
    border-bottom-left-radius: 4px;
    font-style: italic;
    box-shadow: 0 4px 15px rgba(255, 107, 107, 0.3);
}

.input-area {
    background: rgba(0, 0, 0, 0.4);
    backdrop-filter: blur(10px);
    padding: 20px 30px;
    border-top: 1px solid rgba(255, 255, 255, 0.1);
}

.input-row {
    display: flex;
    gap: 12px;
    align-items: center;
}

.input-row input {
    flex: 1;
    background: rgba(255, 255, 255, 0.08);
    border: 1px solid rgba(255, 255, 255, 0.2);
    border-radius: 12px;
    padding: 14px 20px;
    font-size: 16px;
    color: white;
    outline: none;
    transition: all 0.2s ease;
}

.input-row input::placeholder {
    color: #666;
}

.input-row input:focus {
    border-color: #667eea;
    box-shadow: 0 0 0 3px rgba(102, 126, 234, 0.2);
}

.input-row button {
    background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
    border: none;
    border-radius: 12px;
    padding: 14px 28px;
    font-size: 16px;
    font-weight: 600;
    color: white;
    cursor: pointer;
    transition: all 0.2s ease;
    box-shadow: 0 4px 15px rgba(102, 126, 234, 0.3);
}

.input-row button:hover {
    transform: translateY(-2px);
    box-shadow: 0 6px 20px rgba(102, 126, 234, 0.4);
}

.input-row button:active {
    transform: translateY(0);
}
"#;

pub fn App() -> Element {
    let license_state = use_context_provider(LicenseState::new);
    let _registry_auth = use_context_provider(RegistryAuthState::new);

    use_effect(move || {
        let mut license_state = license_state.clone();
        spawn(async move {
            if license_state.has_license() && license_state.needs_revalidation() {
                license_state.validate().await;
            }
        });
    });

    rsx! {
        style { "{GLOBAL_STYLE}" }
        Router::<Route> {}
    }
}
