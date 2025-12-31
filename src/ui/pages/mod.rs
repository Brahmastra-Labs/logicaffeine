pub mod home;
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
pub mod profile;

pub use home::Home;
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
pub use profile::Profile;
