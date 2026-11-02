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
//! ```no_run
//! use logicaffeine_system::file;
//!
//! # fn main() -> Result<(), String> {
//! // Write a file
//! file::write("data.txt".to_string(), "Hello, World!".to_string())?;
//!
//! // Read it back
//! let content = file::read("data.txt".to_string())?;
//! assert_eq!(content, "Hello, World!");
//! # Ok(())
//! # }
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
/// ```no_run
/// use logicaffeine_system::file;
///
/// match file::read("config.json".to_string()) {
///     Ok(content) => println!("Config: {}", content),
///     Err(e) => eprintln!("Error: {}", e),
/// }
/// ```
pub fn read(path: String) -> Result<String, String> {
    if let Some(sealed) = crate::testworld::intercept_read(&path) {
        return sealed;
    }
    fs::read_to_string(&path).map_err(|e| format!("Failed to read '{}': {}", path, e))
}

/// Reads a file's raw bytes. The total counterpart of the trapping `readBytes` builtin.
///
/// ★ ONE IMPLEMENTATION AND ONE MESSAGE, so the tiers cannot drift. The interpreter and the
/// AOT-Rust lowering both call THIS — the alternative was two `std::fs::read` calls with two
/// hand-written format strings, which is how an error message comes to differ between the tier
/// an author tests on and the tier they ship.
pub fn read_bytes(path: String) -> Result<Vec<u8>, String> {
    fs::read(&path).map_err(|e| format!("cannot read '{}': {}", path, e))
}

/// Reads a file of little-endian IEEE-754 binary64 values, eight bytes a value in order: the numbers a program reads
/// at run time rather than carries as code (`readReals`; a model's weights). The interpreter and the AOT-Rust lowering
/// both call this, so the read and its refusals are one. A file that is not a whole number of values is refused.
pub fn read_reals(path: String) -> Result<Vec<f64>, String> {
    let bytes = fs::read(&path).map_err(|e| format!("readReals: cannot read '{}': {}", path, e))?;
    if bytes.len() % 8 != 0 {
        return Err(format!("readReals: '{}' holds {} bytes, not a whole number of 8-byte Reals", path, bytes.len()));
    }
    Ok(bytes.chunks_exact(8).map(|b| f64::from_le_bytes(b.try_into().expect("an 8-byte chunk"))).collect())
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
/// ```no_run
/// use logicaffeine_system::file;
///
/// # fn main() -> Result<(), String> {
/// file::write("output.txt".to_string(), "Result: 42".to_string())?;
/// # Ok(())
/// # }
/// ```
pub fn write(path: String, content: String) -> Result<(), String> {
    if let Some(sealed) = crate::testworld::intercept_write(&path, &content) {
        return sealed;
    }
    fs::write(&path, &content).map_err(|e| format!("Failed to write '{}': {}", path, e))
}
