#![cfg_attr(docsrs, feature(doc_cfg))]

//! # logicaffeine-lsp
//!
//! Language Server Protocol implementation providing IDE integration for LogicAffeine.
//!
//! This crate implements a complete LSP server that enables rich code intelligence in any LSP-compatible editor,
//! including diagnostics, completion, hover documentation, refactoring, and more.
//!
//! ## Quick Start
//!
//! Install the language server binary:
//!
//! ```bash
//! cargo install logicaffeine-lsp
//! ```
//!
//! Configure your editor to use `logicaffeine-lsp` as the language server for `.logos` files.
//! See the [Editor Integration](#editor-integration) section for specific setup instructions.
//!
//! ## Features
//!
//! The language server provides 14 LSP features organized into four categories:
//!
//! ### Code Intelligence
//!
//! | Feature | Description |
//! |---------|-------------|
//! | **Diagnostics** | Real-time syntax and semantic error detection with actionable error messages |
//! | **Hover** | Type information, documentation, and definition context on hover |
//! | **Semantic Tokens** | Syntax highlighting based on semantic meaning (variables, keywords, types) |
//!
//! ### Navigation
//!
//! | Feature | Description |
//! |---------|-------------|
//! | **Go to Definition** | Jump to where a variable, function, or type is defined |
//! | **Find References** | Find all usages of a symbol across the codebase |
//! | **Document Symbols** | Outline view showing all declarations in the file |
//! | **Code Lens** | Inline reference counts and navigation actions |
//!
//! ### Refactoring
//!
//! | Feature | Description |
//! |---------|-------------|
//! | **Rename** | Safely rename symbols across all references with preview |
//! | **Code Actions** | Quick fixes and refactoring suggestions (e.g., "Extract to function") |
//!
//! ### Editing
//!
//! | Feature | Description |
//! |---------|-------------|
//! | **Completion** | Context-aware autocomplete for keywords, variables, and functions |
//! | **Signature Help** | Parameter hints for function calls |
//! | **Inlay Hints** | Inline type annotations and parameter names |
//!
//! ### Display
//!
//! | Feature | Description |
//! |---------|-------------|
//! | **Folding Ranges** | Collapsible code regions for functions, blocks, and comments |
//! | **Formatting** | Automatic code formatting with consistent style |
//!
//! ## Editor Integration
//!
//! ### VSCode
//!
//! Add to your `settings.json`:
//!
//! ```json
//! {
//!   "logicaffeine.lsp.serverPath": "/path/to/logicaffeine-lsp",
//!   "logicaffeine.lsp.trace.server": "verbose"
//! }
//! ```
//!
//! The official VSCode extension is available at `editors/vscode/logicaffeine/`.
//!
//! ### Neovim
//!
//! Using `nvim-lspconfig`:
//!
//! ```lua
//! local lspconfig = require('lspconfig')
//! local configs = require('lspconfig.configs')
//!
//! if not configs.logicaffeine_lsp then
//!   configs.logicaffeine_lsp = {
//!     default_config = {
//!       cmd = { 'logicaffeine-lsp' },
//!       filetypes = { 'logos' },
//!       root_dir = lspconfig.util.root_pattern('.git', 'Project.toml'),
//!       settings = {},
//!     },
//!   }
//! end
//!
//! lspconfig.logicaffeine_lsp.setup{}
//! ```
//!
//! ### Emacs (lsp-mode)
//!
//! ```elisp
//! (require 'lsp-mode)
//! (add-to-list 'lsp-language-id-configuration '(logos-mode . "logos"))
//! (lsp-register-client
//!  (make-lsp-client :new-connection (lsp-stdio-connection "logicaffeine-lsp")
//!                   :major-modes '(logos-mode)
//!                   :server-id 'logicaffeine-lsp))
//! ```
//!
//! ### Emacs (eglot)
//!
//! ```elisp
//! (require 'eglot)
//! (add-to-list 'eglot-server-programs '(logos-mode . ("logicaffeine-lsp")))
//! ```
//!
//! ### Sublime Text
//!
//! Install the LSP package, then add to your LSP settings:
//!
//! ```json
//! {
//!   "clients": {
//!     "logicaffeine-lsp": {
//!       "enabled": true,
//!       "command": ["logicaffeine-lsp"],
//!       "selector": "source.logos"
//!     }
//!   }
//! }
//! ```
//!
//! ## Architecture
//!
//! ```text
//! LSP Client (Editor)
//!      │
//!      ▼
//! ┌─────────────────────────────────────────┐
//! │     Language Server (tower-lsp)         │
//! │  ┌─────────────────────────────────┐   │
//! │  │   Document State Management      │   │
//! │  │   (incremental sync, indexing)   │   │
//! │  └─────────────────────────────────┘   │
//! └──────────────┬──────────────────────────┘
//!                │
//!                ▼
//! ┌─────────────────────────────────────────┐
//! │     LogicAffeine Pipeline               │
//! │  ┌──────┐  ┌────────┐  ┌──────────┐   │
//! │  │Lexer │→ │ Parser │→ │ Analysis │   │
//! │  └──────┘  └────────┘  └──────────┘   │
//! └─────────────────────────────────────────┘
//!                │
//!                ▼
//! ┌─────────────────────────────────────────┐
//! │          LSP Features                    │
//! │  Diagnostics, Hover, Completion, etc.   │
//! └─────────────────────────────────────────┘
//! ```
//!
//! The server integrates LogicAffeine's compilation pipeline with the LSP protocol:
//!
//! 1. **[`server`]** - Tower-LSP server handling client communication
//! 2. **[`state`]** - Document state management with incremental updates
//! 3. **[`pipeline`]** - Compilation pipeline integration (lexer, parser, analysis)
//! 4. **[`index`]** - Symbol indexing for cross-file references
//! 5. **Feature modules** - Individual LSP capabilities ([`diagnostics`], [`hover`], [`completion`], etc.)
//!
//! ## Modules
//!
//! - [`server`] - Main LSP server implementation using tower-lsp
//! - [`state`] - Document state and workspace management
//! - [`document`] - Individual document handling with incremental sync
//! - [`pipeline`] - Integration with LogicAffeine compilation pipeline
//! - [`index`] - Symbol indexing and cross-reference tracking
//! - [`line_index`] - Line/column to byte offset conversion
//!
//! ### Feature Modules
//!
//! - [`diagnostics`] - Error and warning reporting
//! - [`semantic_tokens`] - Semantic syntax highlighting
//! - [`document_symbols`] - Outline and breadcrumb navigation
//! - [`definition`] - Go to definition
//! - [`references`] - Find all references
//! - [`hover`] - Documentation on hover
//! - [`completion`] - Autocomplete
//! - [`signature_help`] - Function parameter hints
//! - [`code_actions`] - Quick fixes and refactorings
//! - [`rename`] - Symbol renaming
//! - [`inlay_hints`] - Inline type hints
//! - [`code_lens`] - Inline actions and reference counts
//! - [`folding`] - Code folding ranges
//! - [`formatting`] - Code formatting
//!
//! ## Testing
//!
//! The crate includes 179 comprehensive tests covering all LSP features:
//!
//! ```bash
//! cargo test -p logicaffeine-lsp --lib
//! ```
//!
//! ## Performance
//!
//! - **Incremental sync**: Only re-analyzes changed documents
//! - **Parallel analysis**: Uses `DashMap` for concurrent document processing
//! - **Lazy indexing**: Symbol index built on-demand
//! - **Optimized for REPL**: Workspace state persists across document changes
//!
//! ## License
//!
//! Business Source License 1.1 (BUSL-1.1)
//!
//! - **Free** for individuals and organizations with <25 employees
//! - **Commercial license** required for organizations with 25+ employees offering Logic Services
//! - **Converts to MIT** on December 24, 2029
//!
//! See [LICENSE](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md) for full terms.

pub mod line_index;
pub mod state;
pub mod document;
pub mod pipeline;
pub mod diagnostics;
pub mod semantic_tokens;
pub mod document_symbols;
pub mod index;
pub mod definition;
pub mod hover;
pub mod completion;
pub mod references;
pub mod signature_help;
pub mod code_actions;
pub mod rename;
pub mod folding;
pub mod inlay_hints;
pub mod code_lens;
pub mod formatting;
pub mod server;
