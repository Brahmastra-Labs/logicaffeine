//! Reusable UI components.
//!
//! This module contains all Dioxus components used throughout the application.
//! Components are organized by function:
//!
//! # Navigation
//! - [`main_nav`] - Main site navigation with links
//!
//! # Learning Interface
//! - [`learn_roadmap`] - The Learn roadmap: eras, modules and lessons in order
//! - [`symbol_pad`] - The symbol buttons that type into an answer field
//! - [`socratic_guide`] - Hint display with progressive revelation
//! - [`symbol_dictionary`] - Logic symbol quick reference
//!
//! # Studio (Playground)
//! - [`file_browser`] - Virtual file system tree
//! - [`collapsible`] - A titled block of a panel the reader opens and closes
//! - [`chevron`] - The drawn chevron that says whether a folder or block is open
//! - [`tree`] - The tree the file browser and the Syntax Tree are drawn with
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
//! - [`chat`] - Chat message display

pub mod ast_tree;
pub mod chat;
pub mod exercise_block;
pub mod footer;
pub mod guide_code_block;
pub mod guide_sidebar;
pub mod guide_snippet;
pub mod icon;
pub mod inline_editor;
pub mod input;
pub mod katex;
pub mod learn_roadmap;
pub mod lexicon_gate;
pub mod logic_output;
pub mod main_nav;
pub mod misconception_box;
pub mod recall_block;
pub mod socratic_guide;
pub mod symbol_dictionary;
pub mod symbol_pad;

// Studio components
pub mod chevron;
pub mod code_editor;
pub mod collapsible;
pub mod context_view;
pub mod debug_drawer;
pub mod file_browser;
pub mod formula_editor;
pub mod hover_hint;
pub mod indent;
pub mod problems;
pub mod proof_panel;
pub mod repl_output;
pub mod status_bar;
pub mod tree;
pub mod verilog;
pub mod vernacular;
