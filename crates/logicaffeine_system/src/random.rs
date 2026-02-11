//! Random Number Generation
//!
//! Provides random number generation using thread-local RNG with
//! cryptographically secure seeding from system entropy.
//!
//! # Thread Safety
//!
//! Uses thread-local RNG via `rand::thread_rng()`. Each thread gets its
//! own independent RNG instance, so this is safe to call from any thread
//! without synchronization.
//!
//! # Platform Support
//!
//! - **Native**: Uses system entropy for seeding (getrandom)
//! - **WASM**: Not available (module not compiled for wasm32)
//!
//! # Example
//!
//! ```
//! use logicaffeine_system::random;
//!
//! let dice_roll = random::randomInt(1, 6);
//! assert!((1..=6).contains(&dice_roll));
//!
//! let probability = random::randomFloat();
//! assert!((0.0..1.0).contains(&probability));
//! ```

use rand::Rng;

/// Generates a random integer in an inclusive range.
///
/// # Arguments
///
/// * `min` - Minimum value (inclusive)
/// * `max` - Maximum value (inclusive)
///
/// # Returns
///
/// A random integer in the range `[min, max]`.
///
/// # Panics
///
/// Panics if `min > max`.
///
/// # Example
///
/// ```
/// use logicaffeine_system::random;
///
/// let dice = random::randomInt(1, 6); // 1, 2, 3, 4, 5, or 6
/// assert!((1..=6).contains(&dice));
/// ```
#[allow(non_snake_case)]
pub fn randomInt(min: i64, max: i64) -> i64 {
    let mut rng = rand::thread_rng();
    rng.gen_range(min..=max)
}

/// Generates a random floating-point number.
///
/// # Returns
///
/// A random float in the range `[0.0, 1.0)` (includes 0.0, excludes 1.0).
///
/// # Example
///
/// ```
/// use logicaffeine_system::random;
///
/// let chance = random::randomFloat();
/// assert!((0.0..1.0).contains(&chance));
/// ```
#[allow(non_snake_case)]
pub fn randomFloat() -> f64 {
    let mut rng = rand::thread_rng();
    rng.gen()
}
