//! Environment module for LOGOS standard library.

use std::env as std_env;

/// Get an environment variable by name.
pub fn get(key: String) -> Option<String> {
    std_env::var(&key).ok()
}

/// Get command-line arguments.
pub fn args() -> Vec<String> {
    std_env::args().collect()
}
