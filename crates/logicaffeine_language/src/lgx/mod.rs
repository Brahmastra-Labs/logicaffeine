//! LGX — the LOGOS web framework's template layer: real HTML with Jinja-style `{…}` tags,
//! component-scoped CSS, and the structural checks a template needs before any LOGOS
//! expression inside it is parsed.
//!
//! Three pure, grammar-agnostic stages, each locked on its own:
//!
//! - [`html`] — the flat tokenizer: tags, attributes, text, `{…}` brace groups, entities,
//!   void elements, comments, and the refusals (`<script>`, `<style>`, a doctype);
//! - [`template`] — the tree: nesting, `{if}`/`{for each}`/`{inspect}`/`{let}`/`{extends}`/
//!   `{block}`/`{children}`/`{raw}`, holes with filters, components, directives, whitespace;
//!   every LOGOS expression stays text + offset for the real expression parser;
//! - [`css`] — component scoping by a real tokenizer (`.card h1` → `.card h1.lgx-todos`).
//!
//! The parser (`parser/component.rs`) turns a [`template::TNode`] tree into the view IR
//! ([`crate::ast::view`]) by sub-lexing each expression at its recorded offset.

pub mod check;
pub mod css;
pub mod deps;
pub mod entities;
pub mod html;
pub mod raw;
pub mod selector;
pub mod template;
