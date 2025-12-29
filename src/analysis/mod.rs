pub mod registry;
pub mod discovery;
pub mod dependencies;
pub mod escape;
pub mod ownership;

pub use registry::{TypeRegistry, TypeDef};
pub use discovery::DiscoveryPass;
pub use dependencies::{scan_dependencies, Dependency};
pub use escape::{EscapeChecker, EscapeError, EscapeErrorKind};
pub use ownership::{OwnershipChecker, OwnershipError, OwnershipErrorKind, VarState};

#[cfg(not(target_arch = "wasm32"))]
pub use discovery::discover_with_imports;
