//! The visitor's system, read from what their browser says, so the front page shows the install that
//! runs on it.

use crate::ui::pages::landing::content::Install;

/// What the browser says about the machine it runs on.
pub struct Seen<'a> {
    /// `navigator.userAgentData.platform`, where the browser offers it (Chromium).
    pub ua_platform: Option<&'a str>,
    /// `navigator.platform`.
    pub platform: &'a str,
    pub user_agent: &'a str,
    pub max_touch_points: u32,
}

/// The systems the front page tells apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum System {
    MacOs,
    Linux,
    Windows,
    ChromeOs,
    /// A phone or a tablet.
    Phone,
    Unknown,
}

impl System {
    pub const ALL: &'static [System] = &[System::MacOs, System::Linux, System::Windows, System::ChromeOs, System::Phone, System::Unknown];

    /// The name the fixture, the inline script and `<html data-system>` use.
    pub fn name(self) -> &'static str {
        match self {
            System::MacOs => "macos",
            System::Linux => "linux",
            System::Windows => "windows",
            System::ChromeOs => "chromeos",
            System::Phone => "phone",
            System::Unknown => "unknown",
        }
    }
}

/// The system a browser runs on. Phones and tablets first: Android says Linux, and an iPad asking for
/// the desktop site says it is a Mac with a touch screen. [`SYSTEM_OF_JS`] is this function in
/// JavaScript, for the prerendered page.
pub fn system_of(seen: &Seen) -> System {
    let ua = seen.user_agent;
    let platform = seen.platform;
    let ua_platform = seen.ua_platform.unwrap_or("");
    let says = |word: &str| ua.contains(word);
    let starts = |prefix: &str| platform.starts_with(prefix);
    if says("iPhone")
        || says("iPad")
        || says("iPod")
        || says("Android")
        || ua_platform == "Android"
        || ua_platform == "iOS"
        || starts("iPhone")
        || starts("iPad")
        || starts("iPod")
        || (platform == "MacIntel" && seen.max_touch_points > 1)
    {
        System::Phone
    } else if says("CrOS") || ua_platform == "Chrome OS" || ua_platform == "ChromeOS" {
        System::ChromeOs
    } else if ua_platform == "Windows" || starts("Win") || says("Windows") {
        System::Windows
    } else if ua_platform == "macOS" || starts("Mac") || says("Macintosh") {
        System::MacOs
    } else if ua_platform == "Linux" || starts("Linux") || says("X11") || says("Linux") {
        System::Linux
    } else {
        System::Unknown
    }
}

/// The install that runs on `system`: PowerShell on Windows, the shell script everywhere else. A
/// phone's reader pastes into an agent on a Mac or a Linux machine, or into a cloud agent, which runs
/// in a Linux sandbox.
pub fn install_for(system: System) -> Install {
    match system {
        System::Windows => Install::Windows,
        System::MacOs | System::Linux | System::ChromeOs | System::Phone | System::Unknown => Install::Unix,
    }
}

/// Where the page remembers the install a visitor chose with the switch, in their browser alone.
pub const REMEMBERED: &str = "logos-install";

/// [`system_of`] in JavaScript: `systemOf(userAgent, platform, uaPlatform, maxTouchPoints)` returns
/// [`System::name`]. `landing_page::the_os_script_reads_the_system_as_the_app_does` holds the two to
/// the same answers for every visitor of the fixture.
pub const SYSTEM_OF_JS: &str = "function systemOf(ua, platform, uaPlatform, touch) { \
var says = function (w) { return ua.indexOf(w) !== -1; }; \
var starts = function (p) { return platform.indexOf(p) === 0; }; \
if (says('iPhone') || says('iPad') || says('iPod') || says('Android') || uaPlatform === 'Android' || uaPlatform === 'iOS' || starts('iPhone') || starts('iPad') || starts('iPod') || (platform === 'MacIntel' && touch > 1)) return 'phone'; \
if (says('CrOS') || uaPlatform === 'Chrome OS' || uaPlatform === 'ChromeOS') return 'chromeos'; \
if (uaPlatform === 'Windows' || starts('Win') || says('Windows')) return 'windows'; \
if (uaPlatform === 'macOS' || starts('Mac') || says('Macintosh')) return 'macos'; \
if (uaPlatform === 'Linux' || starts('Linux') || says('X11') || says('Linux')) return 'linux'; \
return 'unknown'; }";

/// The script the prerendered front page runs before its hero paints: `<html>` gets the visitor's
/// system (`data-system`) and the install to show (`data-os`), the one they chose last time if they
/// chose one, so the page shows the right prompt with no flash and no jump.
pub fn os_script() -> String {
    format!(
        "(function () {{ {SYSTEM_OF_JS} var n = navigator; \
var system = systemOf(n.userAgent || '', n.platform || '', (n.userAgentData && n.userAgentData.platform) || '', n.maxTouchPoints || 0); \
var chosen = null; try {{ chosen = localStorage.getItem('{REMEMBERED}'); }} catch (e) {{}} \
var root = document.documentElement; root.setAttribute('data-system', system); \
root.setAttribute('data-os', chosen === 'windows' || chosen === 'unix' ? chosen : (system === 'windows' ? 'windows' : 'unix')); }})();"
    )
}

/// The install the page shows: what `<html data-os>` says, else what the app reads of the browser and
/// the visitor's last choice, marking `<html>` with it as the inline script would have. Off the
/// browser, the shell install.
pub fn shown_install() -> Install {
    #[cfg(target_arch = "wasm32")]
    {
        let window = web_sys::window();
        let Some(root) = window.as_ref().and_then(|w| w.document()).and_then(|d| d.document_element()) else {
            return Install::Unix;
        };
        if let Some(install) = root.get_attribute("data-os").as_deref().and_then(Install::from_name) {
            return install;
        }
        let navigator = window.as_ref().map(|w| w.navigator());
        let read = |of: &wasm_bindgen::JsValue, name: &str| js_sys::Reflect::get(of, &name.into()).ok().and_then(|v| v.as_string());
        let user_agent = navigator.as_ref().and_then(|n| read(n, "userAgent")).unwrap_or_default();
        let platform = navigator.as_ref().and_then(|n| read(n, "platform")).unwrap_or_default();
        let ua_platform = navigator
            .as_ref()
            .and_then(|n| js_sys::Reflect::get(n, &"userAgentData".into()).ok())
            .filter(|data| data.is_object())
            .and_then(|data| read(&data, "platform"));
        let max_touch_points = navigator.as_ref().map_or(0, |n| n.max_touch_points().max(0) as u32);
        let system = system_of(&Seen { ua_platform: ua_platform.as_deref(), platform: &platform, user_agent: &user_agent, max_touch_points });
        let chosen = window.as_ref().and_then(|w| w.local_storage().ok().flatten()).and_then(|s| s.get_item(REMEMBERED).ok().flatten());
        let install = chosen.as_deref().and_then(Install::from_name).unwrap_or(install_for(system));
        let _ = root.set_attribute("data-system", system.name());
        let _ = root.set_attribute("data-os", install.name());
        install
    }
    #[cfg(not(target_arch = "wasm32"))]
    Install::Unix
}

/// Show `install`'s prompt, and remember the choice in this browser when it lets the page.
pub fn choose_install(install: Install) {
    #[cfg(target_arch = "wasm32")]
    {
        let window = web_sys::window();
        if let Some(root) = window.as_ref().and_then(|w| w.document()).and_then(|d| d.document_element()) {
            let _ = root.set_attribute("data-os", install.name());
        }
        if let Some(storage) = window.as_ref().and_then(|w| w.local_storage().ok().flatten()) {
            let _ = storage.set_item(REMEMBERED, install.name());
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = install;
}

#[cfg(test)]
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/platform/tests.rs"]
mod tests;
