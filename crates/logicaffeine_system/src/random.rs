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
//! ```rust,ignore
//! use logicaffeine_system::random;
//!
//! let dice_roll = random::randomInt(1, 6);
//! let probability = random::randomFloat();
//!
//! if probability < 0.5 {
//!     println!("Heads!");
//! } else {
//!     println!("Tails!");
//! }
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
/// ```rust,ignore
/// let dice = random::randomInt(1, 6); // 1, 2, 3, 4, 5, or 6
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
/// ```rust,ignore
/// let chance = random::randomFloat();
/// if chance < 0.1 {
///     println!("Critical hit! (10% chance)");
/// }
/// ```
#[allow(non_snake_case)]
pub fn randomFloat() -> f64 {
    let mut rng = rand::thread_rng();
    rng.gen()
}
