//! A real browser as a page tier: headless Chrome, driven over the DevTools protocol.
//!
//! [`Browser::launch`] starts Chrome with `--remote-debugging-pipe` — the protocol travels over
//! two pipes the harness owns, so no port is opened and nothing else can attach. [`Browser::open`]
//! serves a [`PageBundle`] from a loopback HTTP server and loads it in a fresh target inside a
//! browser context of its own (no cookies, storage or cache shared with any other page), then
//! returns a [`Page`] once the page's module has run `main` and committed its first paint.
//!
//! What a [`Page`] offers is the whole driving surface a page tier needs:
//!
//! * real input — [`Page::click`], [`Page::type_into`], [`Page::press`] dispatch trusted mouse and
//!   keyboard events through the browser's input pipeline, so default actions (a submit button
//!   submitting, Enter submitting a form) happen exactly as they do for a visitor; [`Page::choose`]
//!   and [`Page::submit`] use the DOM's own `change`/`requestSubmit` paths;
//! * [`Page::snapshot`] — the document as a canonical tree, title, URL and focused element;
//! * [`Page::import_trace`] — every host import the module called and every export the host
//!   called, in order, recorded by wrapping `WebAssembly.instantiate` (the page's own runtime is
//!   untouched);
//! * [`Page::advance`] — the page's clock under virtual time: paused from the moment the page is
//!   ready, moving only when a test moves it;
//! * [`Page::answer`] — network stubs: every request outside the bundle is intercepted, answered
//!   when a stub matches and otherwise HELD, so an unanswered request stays pending as it would
//!   on a network that has not replied;
//! * navigation — a document the page asks the browser for (a link it follows, a form it submits,
//!   a `location` it assigns) is let in only once the document that asked has reported everything
//!   it will, and every act, settle and read is made in the document the page came to rest in,
//!   never in one half-replaced;
//! * [`Page::eval`] — anything else, as JavaScript.
//!
//! One browser serves a whole test binary ([`shared`]); every [`Page`] is its own target and is
//! closed when dropped. Every wait has a deadline and fails loudly, naming what it waited for.

mod bundle;
mod cdp;
mod page;
#[cfg(unix)]
mod process;
mod remote_page;
mod server;
mod snapshot;

pub use bundle::PageBundle;
pub use page::{
    Answer, CallKind, ConsoleLine, DomCounters, HostCall, OpenOptions, Page, PageRequest,
};
pub use remote_page::ChromeLauncher;
pub use server::RelayHub;
pub use snapshot::{DomNode, Snapshot};

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// How long any single protocol command may take before the harness gives up on it.
pub(crate) const COMMAND_TIMEOUT: Duration = Duration::from_secs(20);

/// How long a command that hands control to the page's own code may take: a trusted input event, or a harness
/// action that fires one, returns only once the page has handled it, so its wait is the page's work, which takes as
/// long as the program's handler runs and the machine allows. Still bounded, so a page that never returns fails.
pub(crate) const PAGE_WORK_TIMEOUT: Duration = Duration::from_secs(300);

/// Why a browser operation failed.
#[derive(Debug)]
pub enum BrowserError {
    /// Chrome could not be started.
    Launch(String),
    /// The DevTools pipe closed; the browser is gone.
    Closed(String),
    /// A command came back with an error.
    Protocol { method: String, message: String },
    /// Something did not happen in time.
    Timeout { what: String, after: Duration },
    /// The page threw, or a script the harness ran against it did.
    Page(String),
    /// The page or its bundle could not be prepared.
    Setup(String),
}

impl std::fmt::Display for BrowserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BrowserError::Launch(m) => write!(f, "Chrome did not start: {m}"),
            BrowserError::Closed(m) => write!(f, "the browser is gone: {m}"),
            BrowserError::Protocol { method, message } => write!(f, "`{method}` failed: {message}"),
            BrowserError::Timeout { what, after } => {
                write!(
                    f,
                    "timed out after {:.1}s waiting for {what}",
                    after.as_secs_f64()
                )
            }
            BrowserError::Page(m) => write!(f, "{m}"),
            BrowserError::Setup(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for BrowserError {}

/// The Chrome (or Chromium) binary this machine has: `CHROME_BIN` when it names a file, otherwise
/// the first of the usual names found on `PATH`.
pub fn locate_chrome() -> Option<PathBuf> {
    if let Some(bin) = std::env::var_os("CHROME_BIN").map(PathBuf::from) {
        if bin.is_file() {
            return Some(bin);
        }
    }
    const NAMES: &[&str] = &[
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "chrome",
    ];
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for name in NAMES {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let app = PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome");
        if app.is_file() {
            return Some(app);
        }
    }
    None
}

/// A running headless Chrome.
pub struct Browser {
    wire: Arc<cdp::Wire>,
    child: Mutex<Option<std::process::Child>>,
    profile: PathBuf,
    version: String,
}

impl Browser {
    /// Start `chrome` with a fresh profile and connect to it.
    #[cfg(unix)]
    pub fn launch(chrome: &Path) -> Result<Browser, BrowserError> {
        let launched = process::launch(chrome)
            .map_err(|e| BrowserError::Launch(format!("{}: {e}", chrome.display())))?;
        let wire = cdp::Wire::start(launched.to_browser, launched.from_browser);
        let mut browser = Browser {
            wire,
            child: Mutex::new(Some(launched.child)),
            profile: launched.profile,
            version: String::new(),
        };
        let version = browser
            .command("Browser.getVersion", serde_json::json!({}))
            .map_err(|e| {
                BrowserError::Launch(format!("{e}\n{}", process::log_tail(&browser.profile)))
            })?;
        browser.version = version["product"].as_str().unwrap_or("Chrome").to_string();
        Ok(browser)
    }

    /// The harness drives Chrome over pipes it passes as fds 3 and 4, which this platform's
    /// process model does not offer.
    #[cfg(not(unix))]
    pub fn launch(chrome: &Path) -> Result<Browser, BrowserError> {
        Err(BrowserError::Launch(format!(
            "{}: the browser harness drives Chrome over `--remote-debugging-pipe` (fds 3 and 4), \
             which it implements for unix hosts only",
            chrome.display()
        )))
    }

    /// `Chrome/154.0.8037.57`.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Open `bundle` in a new page.
    pub fn open(&self, bundle: &PageBundle) -> Result<Page, BrowserError> {
        Page::open(Arc::clone(&self.wire), bundle, &OpenOptions::default())
    }

    /// Open `bundle` with extra setup — scripts that run before the page's own.
    pub fn open_with(
        &self,
        bundle: &PageBundle,
        options: &OpenOptions,
    ) -> Result<Page, BrowserError> {
        Page::open(Arc::clone(&self.wire), bundle, options)
    }

    fn command(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, BrowserError> {
        self.wire.send(None, method, params, COMMAND_TIMEOUT)
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        self.wire
            .send_detached(None, "Browser.close", serde_json::json!({}));
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if std::time::Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(20))
                    }
                    _ => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                }
            }
        }
        let _ = std::fs::remove_dir_all(&self.profile);
    }
}

/// The one browser this process shares, launched from `chrome` on first use.
///
/// A test binary opens many pages and needs one browser; every [`Page`] is an isolated target in
/// a browser context of its own, so sharing the process shares nothing a page can observe. The
/// browser lives until the process exits, and Chrome exits when its pipe closes.
pub fn shared(chrome: &Path) -> Result<&'static Browser, BrowserError> {
    static SHARED: OnceLock<Result<Browser, String>> = OnceLock::new();
    match SHARED.get_or_init(|| Browser::launch(chrome).map_err(|e| e.to_string())) {
        Ok(b) => Ok(b),
        Err(e) => Err(BrowserError::Launch(e.clone())),
    }
}
