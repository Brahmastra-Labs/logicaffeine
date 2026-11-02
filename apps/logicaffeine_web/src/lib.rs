#![doc = include_str!("../README.md")]

// Re-export AstNode from compile crate
pub use logicaffeine_compile::AstNode;

// Game/learning modules
pub mod composer;
pub mod content;
pub mod generator;
pub mod grader;
pub mod guide_review;
pub mod learn_state;
pub mod progress;
pub mod srs;
pub mod storage;
pub mod struggle;

// SEO and sitemap
pub mod sitemap;

// UI module
pub mod ui;

// Re-export the App component
pub use ui::App;
