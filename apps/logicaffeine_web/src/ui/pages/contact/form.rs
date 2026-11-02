//! The contact form's browser half: its fields and their errors, the send, and Turnstile.
//!
//! The rules are [`lead`](super::lead); this is where they meet the page. The form renders
//! everywhere, but only the app can send it: the prerendered button stays disabled until the app
//! takes the page over. Turnstile's script is fetched the first time the form takes focus, never at
//! page load, and its widget is rendered explicitly and shows itself only when it needs the
//! visitor; its theme is passed in script, not as a `data-` attribute.

use dioxus::prelude::*;

use super::content::{SEND, TEAM_EMAIL};
use super::lead::{self, FieldErrors, Outcome};
use crate::ui::router::Route;

/// The Turnstile sitekey. This is Cloudflare's public test key, which always passes; the
/// production key replaces it before launch.
#[cfg(target_arch = "wasm32")]
const TURNSTILE_SITEKEY: &str = "1x00000000000000000000AA";

const EMAIL_ID: &str = "lead-email";
const PHONE_ID: &str = "lead-phone";
const STATUS_ID: &str = "lead-status";

#[derive(Clone, PartialEq)]
enum Status {
    Idle,
    Sending,
    /// Sent, for the address the team will reply to.
    Sent(String),
    Failed,
}

#[component]
pub fn LeadForm() -> Element {
    let mut email = use_signal(String::new);
    let mut phone = use_signal(String::new);
    let mut website = use_signal(String::new);
    let mut errors = use_signal(FieldErrors::default);
    let mut status = use_signal(|| Status::Idle);
    let token = use_signal(|| None::<String>);
    let mut challenge_started = use_signal(|| false);

    use_drop(turnstile::remove);

    let onfocusin = move |_| {
        if !challenge_started() {
            challenge_started.set(true);
            turnstile::start(token);
        }
    };

    let onsubmit = move |event: FormEvent| {
        event.prevent_default();
        if status() == Status::Sending {
            return;
        }
        if let Err(found) = lead::request_body(email.read().as_str(), phone.read().as_str(), "", website.read().as_str()) {
            errors.set(found);
            focus(if found.email.is_some() { EMAIL_ID } else { PHONE_ID });
            return;
        }
        errors.set(FieldErrors::default());
        status.set(Status::Sending);
        if !challenge_started() {
            challenge_started.set(true);
            turnstile::start(token);
        }
        spawn(async move {
            let outcome = match turnstile::token(token).await {
                Some(pass) => match lead::request_body(email.peek().as_str(), phone.peek().as_str(), &pass, website.peek().as_str()) {
                    Ok(body) => send(body).await,
                    Err(found) => Outcome::Invalid(found),
                },
                None => Outcome::Failed,
            };
            turnstile::reset(token);
            match outcome {
                Outcome::Sent => {
                    let address = lead::validate_email(Some(email.peek().as_str())).unwrap_or_else(|_| email.peek().clone());
                    status.set(Status::Sent(address));
                    focus(STATUS_ID);
                }
                Outcome::Invalid(found) => {
                    errors.set(found);
                    status.set(Status::Failed);
                    focus(if found.email.is_some() { EMAIL_ID } else { PHONE_ID });
                }
                Outcome::Failed => {
                    status.set(Status::Failed);
                    focus(STATUS_ID);
                }
            }
        });
    };

    let found = errors();
    let sending = status() == Status::Sending;
    let sent = matches!(status(), Status::Sent(_));
    let email_describedby = found.email.map(|_| "lead-email-error");
    let phone_describedby = if found.phone.is_some() { "lead-phone-hint lead-phone-error" } else { "lead-phone-hint" };
    let status_class = match status() {
        Status::Sent(_) => "lead-status sent",
        Status::Failed => "lead-status failed",
        _ => "lead-status",
    };

    rsx! {
        form {
            class: "lead-form",
            novalidate: true,
            aria_labelledby: "contact-heading",
            onfocusin,
            onsubmit,
            if !sent {
                div { class: "lead-field",
                    label { r#for: EMAIL_ID, "Work email " span { class: "lead-need", "(required)" } }
                    input {
                        id: EMAIL_ID,
                        name: "email",
                        r#type: "email",
                        autocomplete: "email",
                        required: true,
                        aria_invalid: if found.email.is_some() { "true" } else { "false" },
                        aria_describedby: email_describedby,
                        value: "{email}",
                        oninput: move |e| email.set(e.value()),
                    }
                    if let Some(problem) = found.email {
                        p { id: "lead-email-error", class: "lead-error", "{problem.message()}" }
                    }
                }
                div { class: "lead-field",
                    label { r#for: PHONE_ID, "Phone " span { class: "lead-need", "(optional)" } }
                    input {
                        id: PHONE_ID,
                        name: "phone",
                        r#type: "tel",
                        inputmode: "tel",
                        autocomplete: "tel",
                        aria_invalid: if found.phone.is_some() { "true" } else { "false" },
                        aria_describedby: phone_describedby,
                        value: "{phone}",
                        oninput: move |e| phone.set(e.value()),
                    }
                    p { id: "lead-phone-hint", class: "lead-hint", "If you'd like a call back. Never sold, never autodialed." }
                    if let Some(problem) = found.phone {
                        p { id: "lead-phone-error", class: "lead-error", "{problem.message()}" }
                    }
                }
                div { class: "lead-trap", aria_hidden: "true",
                    label { r#for: "lead-website", "Leave this field empty" }
                    input {
                        id: "lead-website",
                        name: "website",
                        r#type: "text",
                        tabindex: "-1",
                        autocomplete: "off",
                        value: "{website}",
                        oninput: move |e| website.set(e.value()),
                    }
                }
                div { id: "lead-turnstile", class: "lead-turnstile" }
                button {
                    class: "contact-btn primary lead-submit",
                    r#type: "submit",
                    disabled: cfg!(not(target_arch = "wasm32")) || sending,
                    if sending { "Sending\u{2026}" } else { "{SEND}" }
                }
            }
            div { id: STATUS_ID, class: status_class, role: "status", aria_live: "polite", tabindex: "-1",
                match status() {
                    Status::Sent(address) => rsx! {
                        p { class: "lead-status-title", "Thanks. The team will reach out to {address}." }
                        p { "An engineer will reply, with a few questions about how you plan to use LOGOS." }
                        p {
                            "Meanwhile, "
                            Link { to: Route::Studio { file: None }, "open Studio" }
                            " or "
                            Link { to: Route::Guide {}, "read the guide" }
                            "."
                        }
                    },
                    Status::Failed => rsx! {
                        p { class: "lead-status-title", "We couldn't send that." }
                        p { "Email " a { href: "mailto:{TEAM_EMAIL}", "{TEAM_EMAIL}" } " instead." }
                    },
                    _ => rsx! {},
                }
            }
            p { class: "lead-privacy",
                "We use these only to reply to you. "
                Link { to: Route::Privacy { section: "pricing-requests".to_string() }, "Privacy policy" }
            }
            p { class: "lead-fallback",
                "Prefer email? "
                a { href: "mailto:{TEAM_EMAIL}", "{TEAM_EMAIL}" }
            }
        }
    }
}

/// Move focus to the element with `id` once the render that shows it has landed.
fn focus(id: &'static str) {
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_futures::spawn_local(async move {
        use wasm_bindgen::JsCast;
        gloo_timers::future::TimeoutFuture::new(0).await;
        if let Some(element) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id(id))
            .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
        {
            let _ = element.focus();
        }
    });
    #[cfg(not(target_arch = "wasm32"))]
    let _ = id;
}

/// POST the request; any failure to reach the worker reads as [`Outcome::Failed`].
async fn send(body: serde_json::Value) -> Outcome {
    #[cfg(target_arch = "wasm32")]
    {
        let Ok(request) = gloo_net::http::Request::post(lead::ENDPOINT).json(&body) else {
            return Outcome::Failed;
        };
        match request.send().await {
            Ok(response) => {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                Outcome::of(status, &text)
            }
            Err(_) => Outcome::Failed,
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = body;
        Outcome::Failed
    }
}

/// Cloudflare Turnstile, loaded on demand. One widget lives in `#lead-turnstile` while the form is
/// on the page; its token lands in the form's `token` signal and is spent by one send.
#[cfg(target_arch = "wasm32")]
mod turnstile {
    use std::cell::{Cell, RefCell};

    use dioxus::prelude::*;
    use js_sys::{Function, Object, Reflect};
    use wasm_bindgen::prelude::*;
    use wasm_bindgen::JsCast;

    use super::TURNSTILE_SITEKEY;
    use crate::ui::pages::contact::lead::TURNSTILE_ACTION;

    const SCRIPT: &str = "https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit";
    const CONTAINER_ID: &str = "lead-turnstile";
    /// How long a send waits for the challenge to pass.
    const WAIT_MS: u32 = 15_000;

    #[derive(Clone, Copy, PartialEq)]
    enum Script {
        Absent,
        Loading,
        Loaded,
        Failed,
    }

    thread_local! {
        static SCRIPT_STATE: Cell<Script> = const { Cell::new(Script::Absent) };
        static WIDGET: RefCell<Option<String>> = const { RefCell::new(None) };
        static FAILED: Cell<bool> = const { Cell::new(false) };
    }

    fn api() -> Option<Object> {
        let window = web_sys::window()?;
        let turnstile = Reflect::get(&window, &JsValue::from_str("turnstile")).ok()?;
        turnstile.dyn_into::<Object>().ok()
    }

    fn call(name: &str, args: &[JsValue]) -> Option<JsValue> {
        let api = api()?;
        let function = Reflect::get(&api, &JsValue::from_str(name)).ok()?.dyn_into::<Function>().ok()?;
        let args: js_sys::Array = args.iter().collect();
        function.apply(&api, &args).ok()
    }

    /// Fetch the script if it is not on the page yet, then render the widget.
    pub fn start(token: Signal<Option<String>>) {
        FAILED.with(|f| f.set(false));
        match SCRIPT_STATE.with(Cell::get) {
            Script::Loaded => render(token),
            Script::Loading => {}
            Script::Absent | Script::Failed => {
                let Some(document) = web_sys::window().and_then(|w| w.document()) else { return };
                let Ok(script) = document.create_element("script") else { return };
                let _ = script.set_attribute("src", SCRIPT);
                let _ = script.set_attribute("async", "");
                let onload = Closure::once_into_js(move || {
                    SCRIPT_STATE.with(|s| s.set(Script::Loaded));
                    render(token);
                });
                let onerror = Closure::once_into_js(move || {
                    SCRIPT_STATE.with(|s| s.set(Script::Failed));
                    FAILED.with(|f| f.set(true));
                });
                let _ = Reflect::set(&script, &JsValue::from_str("onload"), &onload);
                let _ = Reflect::set(&script, &JsValue::from_str("onerror"), &onerror);
                SCRIPT_STATE.with(|s| s.set(Script::Loading));
                if let Some(head) = document.head() {
                    let _ = head.append_child(&script);
                }
            }
        }
    }

    /// Render the widget into the form, unless the form has left the page or has one already.
    fn render(mut token: Signal<Option<String>>) {
        if WIDGET.with(|w| w.borrow().is_some()) {
            return;
        }
        let Some(container) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.get_element_by_id(CONTAINER_ID)) else {
            return;
        };
        let options = Object::new();
        let set = |key: &str, value: &JsValue| {
            let _ = Reflect::set(&options, &JsValue::from_str(key), value);
        };
        set("sitekey", &JsValue::from_str(TURNSTILE_SITEKEY));
        set("action", &JsValue::from_str(TURNSTILE_ACTION));
        set("appearance", &JsValue::from_str("interaction-only"));
        set("theme", &JsValue::from_str("dark"));
        set("size", &JsValue::from_str(if container.client_width() < 300 { "compact" } else { "flexible" }));
        set("refresh-expired", &JsValue::from_str("auto"));
        let passed = Closure::<dyn FnMut(String)>::new(move |pass: String| {
            if let Ok(mut slot) = token.try_write() {
                *slot = Some(pass);
            }
        });
        let cleared = Closure::<dyn FnMut()>::new(move || {
            if let Ok(mut slot) = token.try_write() {
                *slot = None;
            }
        });
        let errored = Closure::<dyn FnMut(JsValue) -> JsValue>::new(move |_code: JsValue| {
            FAILED.with(|f| f.set(true));
            JsValue::TRUE
        });
        set("callback", passed.as_ref());
        set("expired-callback", cleared.as_ref());
        set("error-callback", errored.as_ref());
        passed.forget();
        cleared.forget();
        errored.forget();
        if let Some(id) = call("render", &[container.into(), options.into()]).and_then(|id| id.as_string()) {
            WIDGET.with(|w| *w.borrow_mut() = Some(id));
        }
    }

    /// The token, waiting for the challenge if it has not passed yet; `None` once it failed or
    /// the wait ran out.
    pub async fn token(token: Signal<Option<String>>) -> Option<String> {
        for _ in 0..WAIT_MS / 50 {
            if let Some(pass) = token.peek().clone() {
                return Some(pass);
            }
            if FAILED.with(Cell::get) {
                return None;
            }
            gloo_timers::future::TimeoutFuture::new(50).await;
        }
        None
    }

    /// Spend the token: a Turnstile token is good for one check, so the widget fetches another.
    pub fn reset(mut token: Signal<Option<String>>) {
        token.set(None);
        FAILED.with(|f| f.set(false));
        if let Some(id) = WIDGET.with(|w| w.borrow().clone()) {
            call("reset", &[JsValue::from_str(&id)]);
        }
    }

    /// Take the widget off the page with its form.
    pub fn remove() {
        if let Some(id) = WIDGET.with(|w| w.borrow_mut().take()) {
            call("remove", &[JsValue::from_str(&id)]);
        }
    }
}

/// The server render never talks to Turnstile.
#[cfg(not(target_arch = "wasm32"))]
mod turnstile {
    use dioxus::prelude::*;

    pub fn start(_token: Signal<Option<String>>) {}

    pub async fn token(_token: Signal<Option<String>>) -> Option<String> {
        None
    }

    pub fn reset(_token: Signal<Option<String>>) {}

    pub fn remove() {}
}
