//! The benchmark JIT artifact preserves eager optimizer output but performs
//! native copy-and-patch compilation only when the image executes.

#![cfg(all(target_arch = "x86_64", not(target_os = "windows")))]

use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::{image, ForgeTier};

const LOOP_SUM: &str = include_str!("../../../benchmarks/programs/loop_sum/main.lg");

#[test]
fn optimized_image_runs_on_the_real_native_tier() {
    let bytes = image::build(LOOP_SUM).expect("build image");
    assert!(bytes.starts_with(logicaffeine_compile::vm::binisa::ISA_MAGIC));

    let tier = ForgeTier::new();
    let args = vec!["loop_sum".to_string(), "10000".to_string()];
    let outcome = image::run(&bytes, &args, &tier).expect("run image");
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.lines, ["50005000"]);
    let (attempted, compiled) = tier.region_counts();
    assert!(
        attempted > 0,
        "at least one hot loop was offered to the JIT"
    );
    assert_eq!(compiled, attempted, "every offered loop compiled");
    let execution = tier.execution_summary();
    assert!(
        execution.contains("loops entries=") && !execution.contains("loops entries=0"),
        "a compiled loop actually executed: {execution}"
    );
}

#[test]
fn corrupt_or_version_wrong_images_are_rejected_before_execution() {
    let mut bytes = image::build(LOOP_SUM).expect("build image");
    bytes[0] ^= 0xff;
    let tier = ForgeTier::new();
    assert_eq!(
        image::run(&bytes, &[], &tier).unwrap_err(),
        "invalid or incompatible LOGOS bytecode image"
    );
    assert_eq!(tier.function_counts(), (0, 0));
    assert_eq!(tier.region_counts(), (0, 0));
}

#[test]
fn image_encoding_is_deterministic() {
    let first = image::build(LOOP_SUM).expect("first image");
    let second = image::build(LOOP_SUM).expect("second image");
    assert_eq!(first, second);
}
