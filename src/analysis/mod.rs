pub mod registry;
pub mod discovery;
pub mod dependencies;

pub use registry::{TypeRegistry, TypeDef};
pub use discovery::DiscoveryPass;
pub use dependencies::{scan_dependencies, Dependency};

#[cfg(not(target_arch = "wasm32"))]
pub use discovery::discover_with_imports;
