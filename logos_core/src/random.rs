//! Random number module for LOGOS standard library.

use rand::Rng;

/// Generate a random integer in the range [min, max].
pub fn randomInt(min: i64, max: i64) -> i64 {
    let mut rng = rand::thread_rng();
    rng.gen_range(min..=max)
}

/// Generate a random float in the range [0.0, 1.0).
pub fn randomFloat() -> f64 {
    let mut rng = rand::thread_rng();
    rng.gen()
}
