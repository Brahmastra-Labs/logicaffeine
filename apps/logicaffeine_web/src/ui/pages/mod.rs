//! Application pages and views.
//!
//! Each page corresponds to a route in the application and represents a full-screen
//! view with its own layout and functionality.
//!
//! # Pages
//!
//! ## Learning & Content
//! - [`Learn`] - Main learning interface with curriculum modules
//! - [`Guide`] - Interactive LOGOS language programmer's guide
//! - [`Crates`] - Crate documentation landing page with rustdoc links
//!
//! ## Studio & Tools
//! - [`Studio`] - Multi-mode playground for Logic, Code, and Math
//! - [`Workspace`] - Subject-specific workspace with sidebar and inspector
//!
//! ## Marketing & Info
//! - [`Landing`] - Marketing homepage with feature highlights
//! - [`Pricing`] - Commercial licensing tiers and subscription options
//! - [`Roadmap`] - Development roadmap with milestone progress
//!
//! ## Legal & Account
//! - [`Privacy`] - Privacy policy page
//! - [`Terms`] - Terms of service page
//! - [`Success`] - Post-checkout license activation page
//! - [`Profile`] - User profile and settings
//!
//! ## Package Registry
//! - [`registry`] - Package browsing and details submodule

pub mod landing;
pub mod learn;
// Lesson and Review pages are deprecated - functionality moved to Learn page
// Keeping files for reference during Step 9 refactoring
// pub mod lesson;
// pub mod review;
pub mod pricing;
pub mod privacy;
pub mod registry;
pub mod roadmap;
pub mod success;
pub mod terms;
pub mod workspace;
pub mod studio;
pub mod guide;
pub mod crates;
pub mod profile;
pub mod news;

pub use landing::Landing;
pub use learn::Learn;
pub use pricing::Pricing;
pub use privacy::Privacy;
pub use roadmap::Roadmap;
pub use success::Success;
pub use terms::Terms;
pub use workspace::Workspace;
pub use studio::Studio;
pub use guide::Guide;
pub use crates::Crates;
pub use profile::Profile;
pub use news::{News, NewsArticle};
