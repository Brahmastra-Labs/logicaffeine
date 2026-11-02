pub mod app;
pub mod clipboard;
pub mod components;
pub mod data_fetch;
pub mod drag;
pub mod examples;
pub mod fault;
pub mod fragment;
pub mod hooks;
pub mod inline_markup;
pub mod markdown_blocks;
pub mod pages;
pub mod platform;
pub mod responsive;
pub mod router;
pub mod seo;
pub mod state;

/// The company's contact address, the one every page that offers a way to reach us prints.
pub const CONTACT_EMAIL: &str = "team@datacenter.dev";

pub use app::App;
pub use seo::{JsonLd, JsonLdMultiple};
