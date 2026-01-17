//! Integration test suite for the Logicaffeine system.
//!
//! This crate exists only to run integration tests from the `tests/` directory.
//! All actual test code lives in `tests/*.rs` files.
//!
//! # Test Organization
//!
//! Tests are organized by linguistic complexity phase:
//!
//! | Phase | Topic |
//! |-------|-------|
//! | 1 | Garden path sentences |
//! | 2 | Polarity items |
//! | 3 | Tense and aspect |
//! | 4 | Movement and reciprocals |
//! | 5 | Wh-movement |
//! | 6+ | Advanced phenomena |
//! | 42 | Z3 static verification |
//!
//! # Running Tests
//!
//! ```bash
//! # Run all tests except e2e
//! cargo test -- --skip e2e
//!
//! # Run a specific phase
//! cargo test --test phase1_garden_path
//!
//! # Run with verification (requires Z3)
//! cargo test --features verification
//! ```
