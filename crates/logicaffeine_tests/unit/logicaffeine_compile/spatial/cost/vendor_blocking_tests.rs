use super::vendor_example_blocking;

#[test]
fn the_transcribed_chooser_reproduces_the_examples_logged_parameters() {
    // logs/tt/example__matmul_multicore_reuse__*.log: "per_core_M= 20 -- per_core_N= 4 -- out_subblock_h= 4 -- out_subblock_w= 2"
    assert_eq!(vendor_example_blocking(20, 20, 8, 8), Some((20, 4, 4, 2)));
    // logs/tt/example__matmul_multicore_reuse_mcast__*.log at 2048x1024x512: "per_core_M= 32 -- per_core_N= 4 -- 4 -- 2"
    assert_eq!(vendor_example_blocking(64, 32, 8, 8), Some((32, 4, 4, 2)));
}
