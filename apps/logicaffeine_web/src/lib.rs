#![doc = include_str!("../README.md")]

// Re-export AstNode from compile crate
pub use logicaffeine_compile::AstNode;

// Game/learning modules
pub mod achievements;
pub mod audio;
pub mod content;
pub mod game;
pub mod generator;
pub mod grader;
pub mod learn_state;
pub mod progress;
pub mod srs;
pub mod storage;
pub mod struggle;
pub mod unlock;

// SEO and sitemap
pub mod sitemap;

// UI module
pub mod ui;

// Re-export the App component
pub use ui::App;
