#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod call_hierarchy;
pub mod code_actions;
pub mod code_lens;
pub mod completion;
pub mod definition;
pub mod diagnostics;
pub mod document;
pub mod document_highlights;
pub mod document_symbols;
pub mod flycheck;
pub mod folding;
pub mod formatting;
pub mod hover;
pub mod index;
pub mod inlay_hints;
pub mod line_index;
pub mod pipeline;
pub mod references;
pub mod rename;
pub mod scheduler;
pub mod selection_ranges;
pub mod semantic_tokens;
#[cfg(feature = "server")]
pub mod server;
pub mod signature_help;
pub mod state;
pub mod stdlib_docs;
pub mod teach_md;
pub mod template;
pub mod workspace;

/// The protocol's data types, as this crate's analysis speaks them, for callers that link the
/// analysis directly rather than over the wire.
pub use lsp_types;
