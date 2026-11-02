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
//! - [`Contact`] - The contact page; `/pricing` redirects here
//! - [`Roadmap`] - Development roadmap with milestone progress
//!
//! ## Legal & Account
//! - [`Privacy`] - Privacy policy page
//! - [`Terms`] - Terms of service page
//!
//! ## Package Registry
//! - [`registry`] - Package browsing and details submodule

pub mod benchmarks;
pub mod changelog;
pub mod crates;
pub mod guide;
pub mod hardware_report;
pub mod hardware_spec;
pub mod heal;
pub mod landing;
pub mod learn;
pub mod news;
pub mod pigeonhole_viz;
pub mod contact;
pub mod privacy;
pub mod register_alloc_viz;
pub mod registry;
pub mod roadmap;
pub mod roadmap_data;
pub mod roadmap_history;
pub mod studio;
pub mod studio_actions;
pub mod studio_files;
pub mod studio_layout;
pub mod studio_opening;
pub mod studio_run;
pub mod studio_workbench;
pub mod terms;
pub mod workspace;

pub use benchmarks::Benchmarks;
pub use crates::Crates;
pub use guide::Guide;
pub use landing::Landing;
pub use learn::Learn;
pub use news::{News, NewsArticle};
pub use contact::Contact;
pub use privacy::Privacy;
pub use roadmap::Roadmap;
pub use studio::Studio;
pub use terms::Terms;
pub use workspace::Workspace;
