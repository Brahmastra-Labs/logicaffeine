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
//! ```no_run
//! use logicaffeine_system::time;
//!
//! let start = time::now();
//! time::sleep(1000); // Sleep for 1 second
//! let elapsed = time::now() - start;
//! println!("Elapsed: {}ms", elapsed);
//! ```

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
/// ```
/// use logicaffeine_system::time;
///
/// let timestamp = time::now();
/// assert!(timestamp > 0);
/// ```
pub fn now() -> u64 {
    if let Some(sealed) = crate::testworld::intercept_now() {
        return sealed;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{Duration, SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_millis() as u64
    }
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now() as u64
    }
}

/// Blocks the current thread for the specified duration.
///
/// # Arguments
///
/// * `ms` - Duration to sleep in milliseconds, of any integer type; a negative duration is no wait
///
/// # Note
///
/// This blocks the entire thread. For async code, use `tokio::time::sleep`
/// instead to avoid blocking the executor.
///
/// # Example
///
/// ```no_run
/// use logicaffeine_system::time;
///
/// time::sleep(500); // Sleep for half a second
/// ```
pub fn sleep<T: TryInto<u64>>(ms: T) {
    let ms = ms.try_into().unwrap_or(0);
    if crate::testworld::intercept_sleep(ms) {
        return; // sealed: logical time advanced instantly, no real wait
    }
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::sleep(std::time::Duration::from_millis(ms));
    #[cfg(target_arch = "wasm32")]
    let _ = ms; // the browser cannot block its thread; a live sleep is a no-op
}
