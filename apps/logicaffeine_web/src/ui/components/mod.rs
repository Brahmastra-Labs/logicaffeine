//! Reusable UI components.
//!
//! This module contains all Dioxus components used throughout the application.
//! Components are organized by function:
//!
//! # Navigation
//! - [`app_navbar`] - Top navigation bar for app pages
//! - [`main_nav`] - Main site navigation with links
//!
//! # Learning Interface
//! - [`learn_sidebar`] - Curriculum browser with era/module tree
//! - [`module_tabs`] - Tab bar for switching exercise modes
//! - [`socratic_guide`] - Hint display with progressive revelation
//! - [`vocab_reference`] - Lexicon term lookup
//! - [`symbol_dictionary`] - Logic symbol quick reference
//!
//! # Gamification
//! - [`xp_popup`] - XP gain notification
//! - [`combo_indicator`] - Combo multiplier display
//! - [`streak_display`] - Daily streak counter
//! - [`achievement_toast`] - Achievement unlock overlay
//!
//! # Studio (Playground)
//! - [`mode_toggle`] - Logic/Code/Math mode switcher
//! - [`file_browser`] - Virtual file system tree
//! - [`code_editor`] - Monaco-style code editing
//! - [`formula_editor`] - LaTeX formula input with preview
//! - [`repl_output`] - REPL history display
//! - [`proof_panel`] - Proof tactic interface
//!
//! # Output & Visualization
//! - [`logic_output`] - FOL expression display with formatting
//! - [`ast_tree`] - Interactive syntax tree visualization
//! - [`katex`] - LaTeX math rendering
//! - [`context_view`] - Proof context display
//!
//! # Form Elements
//! - [`input`] - Styled text input
//! - [`editor`] - Multi-line text editor
//! - [`chat`] - Chat message display

pub mod app_navbar;
pub mod chat;
pub mod input;
pub mod editor;
pub mod logic_output;
pub mod ast_tree;
pub mod socratic_guide;
pub mod katex;
pub mod mixed_text;
pub mod xp_popup;
pub mod combo_indicator;
pub mod streak_display;
pub mod achievement_toast;
pub mod mode_selector;
pub mod guide_code_block;
pub mod guide_sidebar;
pub mod learn_sidebar;
pub mod main_nav;
pub mod module_tabs;
pub mod symbol_dictionary;
pub mod vocab_reference;
pub mod footer;
pub mod page_layout;
pub mod icon;
pub mod theme_picker;

// Studio components
pub mod mode_toggle;
pub mod file_browser;
pub mod repl_output;
pub mod context_view;
pub mod symbol_palette;
pub mod formula_editor;
pub mod code_editor;
pub mod proof_panel;
