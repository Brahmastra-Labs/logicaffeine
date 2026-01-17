//! Logicaffeine Web Application
//!
//! Browser-based IDE for LOGOS language learning, built with Dioxus.
//!
//! # Architecture
//!
//! The app is structured around a gamified learning experience that teaches
//! first-order logic through progressive exercises and spaced repetition.
//!
//! ## Data Flow
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                           User Interface                            │
//! │  Landing → Learn → Studio → Profile                                 │
//! └─────────────────────────────────────────────────────────────────────┘
//!                                   │
//!                                   ▼
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                         Content Pipeline                            │
//! │  content (JSON) → generator (templates) → Challenge                 │
//! └─────────────────────────────────────────────────────────────────────┘
//!                                   │
//!                                   ▼
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                        Learning Loop                                │
//! │  Challenge → grader (validation) → game (XP, combos) → feedback     │
//! └─────────────────────────────────────────────────────────────────────┘
//!                                   │
//!                                   ▼
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                       Persistence Layer                             │
//! │  progress → storage (LocalStorage) → srs (scheduling)               │
//! │                   ↓                                                  │
//! │            achievements (unlocks) → unlock (module access)          │
//! └─────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Module Responsibilities
//!
//! | Module | Purpose |
//! |--------|---------|
//! | [`content`] | Loads curriculum from embedded JSON, provides era/module/exercise hierarchy |
//! | [`generator`] | Fills exercise templates with lexicon words, creates [`Challenge`](generator::Challenge) instances |
//! | [`game`] | Manages XP, streaks, combos, level progression, and exercise flow |
//! | [`grader`] | Validates answers with normalization (whitespace, Unicode equivalence) |
//! | [`progress`] | Tracks completed exercises, scores, and review state |
//! | [`srs`] | Implements SM-2 spaced repetition algorithm for review scheduling |
//! | [`achievements`] | Defines and checks achievement conditions, awards badges |
//! | [`unlock`] | State machine for module availability based on prerequisites |
//! | [`storage`] | LocalStorage WASM bindings for progress persistence |
//! | [`audio`] | Sound effect playback via JavaScript interop |
//! | [`learn_state`] | Tab focus and inactivity detection for the Learn page |
//! | [`struggle`] | Detects when users need hints based on attempt patterns |
//! | [`ui`] | Dioxus components, pages, router, and theme system |
//!
//! ## Re-exports
//!
//! - [`AstNode`] - AST representation from the compile crate for syntax visualization
//! - [`App`] - Root Dioxus component for the application

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

// UI module
pub mod ui;

// Re-export the App component
pub use ui::App;
