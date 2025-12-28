//! File I/O module for LOGOS standard library.

use std::fs;

/// Read file contents as text.
pub fn read(path: String) -> Result<String, String> {
    fs::read_to_string(&path).map_err(|e| format!("Failed to read '{}': {}", path, e))
}

/// Write text to a file.
pub fn write(path: String, content: String) -> Result<(), String> {
    fs::write(&path, &content).map_err(|e| format!("Failed to write '{}': {}", path, e))
}
