//! Time Utilities
//!
//! Provides time-related functions for timestamps and delays.
//!
//! # Platform Support
//!
//! - **Native**: Uses `std::time::SystemTime` and `std::thread::sleep`
//! - **WASM**: Not available (use browser APIs via JavaScript interop)
//!
//! # Example
//!
//! ```rust,ignore
//! use logicaffeine_system::time;
//!
//! let start = time::now();
//! time::sleep(1000); // Sleep for 1 second
//! let elapsed = time::now() - start;
//! println!("Elapsed: {}ms", elapsed);
//! ```

use std::time::{SystemTime, UNIX_EPOCH, Duration};
use std::thread;

/// Returns the current time as milliseconds since Unix epoch.
///
/// # Returns
///
/// Milliseconds since January 1, 1970 00:00:00 UTC.
/// Returns 0 if system time is before the Unix epoch (should not happen
/// on properly configured systems).
///
/// # Example
///
/// ```rust,ignore
/// let timestamp = time::now();
/// println!("Current timestamp: {}", timestamp);
/// ```
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64
}

/// Blocks the current thread for the specified duration.
///
/// # Arguments
///
/// * `ms` - Duration to sleep in milliseconds
///
/// # Note
///
/// This blocks the entire thread. For async code, use `tokio::time::sleep`
/// instead to avoid blocking the executor.
///
/// # Example
///
/// ```rust,ignore
/// time::sleep(500); // Sleep for half a second
/// ```
pub fn sleep(ms: u64) {
    thread::sleep(Duration::from_millis(ms));
}
