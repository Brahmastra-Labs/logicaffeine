//! News section for release notes and updates.
//!
//! Provides:
//! - News index page with article list
//! - Individual article pages
//! - Article data model and content

pub mod article;
pub mod data;
pub mod index;

pub use article::NewsArticle;
pub use data::{
    format_tag, get_all_tags, get_article_by_slug, get_articles, get_articles_by_tag, Article,
};
pub use index::News;
