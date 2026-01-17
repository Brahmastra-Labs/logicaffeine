//! Simple File I/O Operations
//!
//! Provides synchronous file read/write operations for simple use cases.
//! For async operations or more advanced file handling, use the [`fs`](crate::fs)
//! module with the [`Vfs`](crate::fs::Vfs) trait.
//!
//! # Features
//!
//! Requires the `persistence` feature.
//!
//! # Platform Support
//!
//! - **Native**: Full support via `std::fs`
//! - **WASM**: Not available (use OPFS via the `fs` module instead)
//!
//! # Example
//!
//! ```rust,ignore
//! use logicaffeine_system::file;
//!
//! // Write a file
//! file::write("data.txt".to_string(), "Hello, World!".to_string())?;
//!
//! // Read it back
//! let content = file::read("data.txt".to_string())?;
//! assert_eq!(content, "Hello, World!");
//! ```

use std::fs;

/// Reads a file as a UTF-8 string.
///
/// # Arguments
///
/// * `path` - Path to the file (relative or absolute)
///
/// # Returns
///
/// The file contents as a string, or an error message describing the failure.
///
/// # Errors
///
/// Returns an error if:
/// - The file doesn't exist
/// - The file can't be read (permissions, I/O error)
/// - The file contents aren't valid UTF-8
///
/// # Example
///
/// ```rust,ignore
/// match file::read("config.json".to_string()) {
///     Ok(content) => println!("Config: {}", content),
///     Err(e) => eprintln!("Error: {}", e),
/// }
/// ```
pub fn read(path: String) -> Result<String, String> {
    fs::read_to_string(&path).map_err(|e| format!("Failed to read '{}': {}", path, e))
}

/// Writes a string to a file.
///
/// Creates the file if it doesn't exist, truncates it if it does.
/// Parent directories must already exist.
///
/// # Arguments
///
/// * `path` - Path to the file (relative or absolute)
/// * `content` - Content to write
///
/// # Errors
///
/// Returns an error if:
/// - Parent directory doesn't exist
/// - File can't be created or written (permissions, I/O error)
///
/// # Example
///
/// ```rust,ignore
/// file::write("output.txt".to_string(), "Result: 42".to_string())?;
/// ```
pub fn write(path: String, content: String) -> Result<(), String> {
    fs::write(&path, &content).map_err(|e| format!("Failed to write '{}': {}", path, e))
}
