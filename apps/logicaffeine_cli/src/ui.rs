//! Terminal output substrate for `largo`.
//!
//! Central home for everything user-facing that is not command logic:
//! the [`CliError`] type rendered by the binary's `main`, the process
//! exit-code conventions, the color/verbosity state shared by every
//! command, and the clap help palette.
//!
//! All commands construct [`CliError`] (directly or via [`CliError::new`] /
//! [`CliError::with_hint`]) instead of ad-hoc string errors when they want
//! a hint line or a non-default exit code. Plain `String` errors continue
//! to work through `Box<dyn Error>` and render without a hint.
//!
//! Every user-visible print goes through [`anstream`], so ANSI styling is
//! automatically stripped on pipes, honored under `--color always`, and
//! silenced by `NO_COLOR` — including styling embedded in engine-rendered
//! messages that pass through here.

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use anstyle::{AnsiColor, Style};
use clap::builder::Styles;

/// Process exit code for success.
pub const EXIT_OK: i32 = 0;
/// Process exit code for a command failure (build error, proof failure,
/// dirty `fmt --check`, parse error, ...).
pub const EXIT_FAILURE: i32 = 1;
/// Process exit code for a usage error (bad arguments, reserved verb).
/// Matches clap's own convention for argument errors.
pub const EXIT_USAGE: i32 = 2;

/// Bold red — the `error:` prefix.
pub const ERROR_STYLE: Style = AnsiColor::Red.on_default().bold();
/// Bold cyan — the `help:` prefix.
pub const HELP_STYLE: Style = AnsiColor::Cyan.on_default().bold();
/// Bold green — phase headers (`Compiling`, `Finished`, ...), cargo-style.
pub const PHASE_STYLE: Style = AnsiColor::Green.on_default().bold();
/// Bold yellow — the `warning:` prefix.
pub const WARN_STYLE: Style = AnsiColor::Yellow.on_default().bold();

/// The cargo help palette for clap: bold-green headers/usage, bold-cyan
/// literals, cyan placeholders.
pub const CLAP_STYLES: Styles = Styles::styled()
    .header(AnsiColor::Green.on_default().bold())
    .usage(AnsiColor::Green.on_default().bold())
    .literal(AnsiColor::Cyan.on_default().bold())
    .placeholder(AnsiColor::Cyan.on_default());

/// The `--color` mode selected on the command line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum ColorMode {
    /// Color when stdout/stderr is a terminal and `NO_COLOR` is unset.
    #[default]
    Auto,
    /// Always emit ANSI, even into pipes (CI logs, `less -R`).
    Always,
    /// Never emit ANSI.
    Never,
}

static QUIET: AtomicBool = AtomicBool::new(false);
static VERBOSITY: AtomicU8 = AtomicU8::new(0);

/// Install the global output state from the parsed CLI flags.
///
/// Called once by `run_cli` before dispatch: records `--quiet`/`--verbose`
/// and sets the process-wide [`anstream`] color choice so every subsequent
/// print (ours or an engine message routed through anstream) obeys the
/// user's `--color` selection.
pub fn init(color: ColorMode, quiet: bool, verbosity: u8) {
    QUIET.store(quiet, Ordering::Relaxed);
    VERBOSITY.store(verbosity, Ordering::Relaxed);
    let choice = match color {
        ColorMode::Auto => anstream::ColorChoice::Auto,
        ColorMode::Always => anstream::ColorChoice::Always,
        ColorMode::Never => anstream::ColorChoice::Never,
    };
    choice.write_global();
}

/// Whether `--quiet` was passed: informational output should be suppressed.
pub fn is_quiet() -> bool {
    QUIET.load(Ordering::Relaxed)
}

/// The `-v` count: `0` normal, `1+` increasingly chatty.
pub fn verbosity() -> u8 {
    VERBOSITY.load(Ordering::Relaxed)
}

/// Print an informational line to stdout unless `--quiet` is active.
pub fn info(msg: impl fmt::Display) {
    if !is_quiet() {
        anstream::println!("{msg}");
    }
}

/// Print a cargo-style phase header (right-aligned bold-green verb) to
/// stderr unless `--quiet` is active.
///
/// ```text
///    Compiling hello v0.1.0 (LOGOS → Rust)
///     Finished dev profile in 3.2s
/// ```
pub fn phase(verb: &str, rest: impl fmt::Display) {
    if !is_quiet() {
        anstream::eprintln!("{PHASE_STYLE}{verb:>12}{PHASE_STYLE:#} {rest}");
    }
}

/// A user-facing CLI error: a message, an optional `help:` hint, and the
/// exit code the process should terminate with.
///
/// The `largo` binary downcasts `Box<dyn Error>` values to this type and
/// renders them as:
///
/// ```text
/// error: <message>
/// help: <hint>
/// ```
#[derive(Debug)]
pub struct CliError {
    /// The primary error message (rendered after `error:`).
    pub message: String,
    /// An optional actionable hint (rendered after `help:`).
    pub hint: Option<String>,
    /// The process exit code to terminate with.
    pub exit_code: i32,
}

impl CliError {
    /// Create an error with the default failure exit code and no hint.
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into(), hint: None, exit_code: EXIT_FAILURE }
    }

    /// Create an error carrying an actionable `help:` hint.
    pub fn with_hint(message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self { message: message.into(), hint: Some(hint.into()), exit_code: EXIT_FAILURE }
    }

    /// Override the exit code (builder style).
    pub fn exit_code(mut self, code: i32) -> Self {
        self.exit_code = code;
        self
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for CliError {}

/// Render any top-level error to stderr in the `error:`/`help:` style and
/// return the exit code the process should use.
///
/// [`CliError`] values carry their own hint and exit code; every other error
/// renders as a bare `error:` line with [`EXIT_FAILURE`].
pub fn render_error(e: &(dyn std::error::Error + 'static)) -> i32 {
    if let Some(cli_err) = e.downcast_ref::<CliError>() {
        anstream::eprintln!("{ERROR_STYLE}error:{ERROR_STYLE:#} {}", cli_err.message);
        if let Some(hint) = &cli_err.hint {
            anstream::eprintln!("{HELP_STYLE}help:{HELP_STYLE:#} {hint}");
        }
        cli_err.exit_code
    } else {
        anstream::eprintln!("{ERROR_STYLE}error:{ERROR_STYLE:#} {e}");
        EXIT_FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_error_defaults_to_failure_exit_code() {
        let e = CliError::new("boom");
        assert_eq!(e.exit_code, EXIT_FAILURE);
        assert!(e.hint.is_none());
        assert_eq!(e.to_string(), "boom");
    }

    #[test]
    fn cli_error_carries_hint_and_custom_code() {
        let e = CliError::with_hint("boom", "try --fix").exit_code(EXIT_USAGE);
        assert_eq!(e.hint.as_deref(), Some("try --fix"));
        assert_eq!(e.exit_code, EXIT_USAGE);
    }

    #[test]
    fn render_error_uses_cli_error_exit_code() {
        let boxed: Box<dyn std::error::Error> =
            Box::new(CliError::new("x").exit_code(EXIT_USAGE));
        assert_eq!(render_error(boxed.as_ref()), EXIT_USAGE);
    }

    #[test]
    fn render_error_defaults_other_errors_to_failure() {
        let boxed: Box<dyn std::error::Error> = "plain".into();
        assert_eq!(render_error(boxed.as_ref()), EXIT_FAILURE);
    }
}
