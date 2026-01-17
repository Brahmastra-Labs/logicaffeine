//! LOGOS CLI (largo) - Standalone binary
//!
//! This is the entry point for the `largo` command-line tool.
//!
//! The binary is a thin wrapper around [`logicaffeine_cli::run_cli`],
//! handling error display and exit codes. All command logic is
//! implemented in the library crate for testability.
//!
//! # Exit Codes
//!
//! - `0` - Success
//! - `1` - Error (message printed to stderr)

fn main() {
    if let Err(e) = logicaffeine_cli::run_cli() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
