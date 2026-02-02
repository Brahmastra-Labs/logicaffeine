//! News section for release notes and updates.
//!
//! Provides:
//! - News index page with article list
//! - Individual article pages
//! - Article data model and content

pub mod index;
pub mod article;
pub mod data;

pub use index::News;
pub use article::NewsArticle;
pub use data::{get_articles, get_article_by_slug, get_all_tags, get_articles_by_tag, format_tag, Article};
