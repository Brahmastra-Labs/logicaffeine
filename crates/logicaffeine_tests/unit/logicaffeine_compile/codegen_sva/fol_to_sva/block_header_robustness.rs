use super::*;

/// A property sentence followed by a `## Theorem`/`## Main` block (multi-section
/// editor content) must still synthesize the property, not fail with a parse error
/// on the trailing block header.
#[test]
fn property_followed_by_a_block_synthesizes() {
    let spec = "Always, if request then eventually grant.\n## Theorem t:\n  It holds.";
    let r = synthesize_sva_from_spec(spec, "clk");
    assert!(r.is_ok(), "expected SVA, got error: {:?}", r.err());
    assert!(r.unwrap().sva_text.contains("property"));
}

/// A property INSIDE a leading block (header first) already works via the parser's
/// leading-header handling — guard that the fix doesn't break it.
#[test]
fn property_inside_a_leading_block_still_works() {
    let spec = "## Hardware\nAlways, if request then eventually grant.";
    let r = synthesize_sva_from_spec(spec, "clk");
    assert!(r.is_ok(), "expected SVA, got error: {:?}", r.err());
}
