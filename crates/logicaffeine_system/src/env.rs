//! Environment Variable and Argument Access
//!
//! Provides access to environment variables and command-line arguments.
//!
//! # Platform Support
//!
//! - **Native**: Full access to system environment
//! - **WASM**: Not available (module not compiled for wasm32)
//!
//! # Example
//!
//! ```rust,ignore
//! use logicaffeine_system::env;
//!
//! // Read environment variable
//! if let Some(home) = env::get("HOME".to_string()) {
//!     println!("Home directory: {}", home);
//! }
//!
//! // Iterate command-line arguments
//! for arg in env::args() {
//!     println!("Argument: {}", arg);
//! }
//! ```

use std::env as std_env;

/// Returns the value of an environment variable.
///
/// # Arguments
///
/// * `key` - The environment variable name
///
/// # Returns
///
/// `Some(value)` if the variable exists and is valid UTF-8, `None` otherwise.
///
/// # Example
///
/// ```rust,ignore
/// let path = env::get("PATH".to_string());
/// ```
pub fn get(key: String) -> Option<String> {
    std_env::var(&key).ok()
}

/// Returns command-line arguments as a vector.
///
/// The first element is the program name (or path), followed by any
/// arguments passed to the program.
///
/// # Returns
///
/// A vector of all command-line arguments.
///
/// # Example
///
/// ```rust,ignore
/// let args = env::args();
/// if args.len() > 1 {
///     println!("First argument: {}", args[1]);
/// }
/// ```
pub fn args() -> Vec<String> {
    std_env::args().collect()
}
