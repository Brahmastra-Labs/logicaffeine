//! Invalid declarations must receive semantic rejection; resource limits remain refusals.
use logicaffeine_kernel::interface::lean as arena;

macro_rules! rejects {
    ($name:ident, $file:literal) => {
        #[test]
        fn $name() {
            let result = arena::check_str(include_str!(concat!("fixtures/lean_arena/", $file)));
            assert!(
                matches!(result, Err(arena::Verdict::Reject(_))),
                "{result:?}"
            );
        }
    };
}
rejects!(
    constructor_parameter_domain_mismatch_is_rejected,
    "050_inductWrongCtorParams.ndjson"
);
rejects!(
    constructor_uniform_result_parameter_mismatch_is_rejected,
    "051_inductWrongCtorResParams.ndjson"
);
rejects!(
    recursive_universe_instance_mismatch_is_rejected,
    "052_inductWrongCtorResLevel.ndjson"
);

#[test]
fn unsupported_export_version_is_still_inconclusive() {
    let result = arena::check_str(r#"{"meta":{"format":{"version":"future"}}}"#);
    assert!(
        matches!(result, Err(arena::Verdict::Decline(_))),
        "{result:?}"
    );
}

#[test]
fn valid_inductives_still_accept() {
    let result = arena::check_str(include_str!("fixtures/lean_arena/037_boolType.ndjson"));
    assert!(result.is_ok(), "{result:?}");
}

rejects!(
    extra_recursor_cannot_add_an_inhabitant_of_false,
    "extra-rec.ndjson"
);
rejects!(
    orphan_recursor_cannot_add_an_inhabitant_of_false,
    "orphan-rec.ndjson"
);
