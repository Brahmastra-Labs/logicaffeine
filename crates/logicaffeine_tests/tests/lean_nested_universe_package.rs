//! The existing pinned witness also crosses bounded canonical source reconstruction.
use logicaffeine_kernel::interface::lean::package::{self, BuildOptions};
use logicaffeine_kernel::interface::lean::receipt::StageStatus;
#[test]
fn equal_nested_universes_survive_portable_package_source_binding() {
    let export = include_str!("fixtures/lean_semantics/LK_NestedMaxLevel.ndjson");
    let package = package::build(export, "LK_NML.Node.width", &BuildOptions::default())
        .unwrap_or_else(|error| panic!("pinned Lean witness: {error:?}"));
    for stage in [
        "translated",
        "primary_check",
        "independent_recheck",
        "assumption_policy",
        "emission_reparsed",
    ] {
        assert_eq!(
            package.receipt.status(stage),
            StageStatus::Passed,
            "{stage}: {:?}",
            package.receipt
        );
    }
    let replay = package::replay(&package.files);
    assert!(
        replay.replayed(),
        "bounded native source reconstruction: {replay:?}"
    );
}
