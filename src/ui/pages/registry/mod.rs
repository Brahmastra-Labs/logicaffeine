//! Phase 39: Package Registry UI
//!
//! Browse, search, and view LOGOS packages.

pub mod browse;
pub mod package_detail;

pub use browse::Registry;
pub use package_detail::PackageDetail;
