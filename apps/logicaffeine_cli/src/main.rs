//! LOGOS CLI (largo) - Standalone binary
//!
//! This is the entry point for the `largo` command-line tool.
//!
//! The binary is exactly [`logicaffeine_cli::largo_main`]: native-tier
//! install, the stack-sized worker running [`logicaffeine_cli::run_cli`],
//! error display and exit codes all live in the library, so the test
//! crate's `largo-under-test` binary is this program.
//!
//! # Exit Codes
//!
//! - `0` - Success
//! - `1` - Error (message printed to stderr)
//! - `2` - Usage error (clap argument errors)
//!
//! Commands may set other codes through
//! [`CliError::exit_code`](logicaffeine_cli::ui::CliError).

fn main() {
    logicaffeine_cli::largo_main();
}
