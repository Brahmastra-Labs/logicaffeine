//! `Word8` and `Word16` are rings — ℤ/2⁸ and ℤ/2¹⁶ — on EVERY tier, exactly like `Word32`/`Word64`.
//!
//! The compiled tier has always lowered them to the `Word8`/`Word16` newtypes, whose arithmetic wraps
//! at their own width. The tree-walker and VM once carried a `word16` in a 32-bit word, so
//! `word16(65535) + word16(3)` printed 65538 in the Studio and 2 in a built binary — the ML-KEM
//! coefficient ring silently computed in the wrong modulus. Each program here is locked to its exact
//! output on the tree-walker, the VM (which must agree), and the compiled binary.

mod common;
use common::{assert_all_tiers, assert_compiled_equals_interpreted_eq, run_all_tiers};

const RING_16: &str = "## Main\n\
Show word16(65535) + word16(3).\n\
Show word16(0) - word16(1).\n\
Show word16(300) * word16(300).\n\
Show word16(70000).\n\
Show word16(0 - 1).\n\
Show word16(65535) / word16(256).\n\
Show word16(65535) % word16(256).\n";
const RING_16_OUT: &str = "2\n65535\n24464\n4464\n65535\n255\n255";

const RING_8: &str = "## Main\n\
Show word8(250) + word8(10).\n\
Show word8(0) - word8(1).\n\
Show word8(16) * word8(16).\n\
Show word8(300).\n\
Show word8(0 - 1).\n\
Show word8(200) / word8(7).\n\
Show word8(200) % word8(7).\n";
const RING_8_OUT: &str = "4\n255\n0\n44\n255\n28\n4";

const BITS_16: &str = "## Main\n\
Let a be word16(61680).\n\
Let b be word16(4080).\n\
Show a & b.\n\
Show a | b.\n\
Show a ^ b.\n\
Show word16(1) shifted left by 15.\n\
Show word16(32768) shifted right by 15.\n\
Show word16(43981) shifted left by 4.\n\
Show rotl(word16(32769), 1).\n\
Show rotr(word16(3), 1).\n";
const BITS_16_OUT: &str = "240\n65520\n65280\n32768\n1\n48336\n3\n32769";

const BITS_8: &str = "## Main\n\
Let a be word8(240).\n\
Let b be word8(60).\n\
Show a & b.\n\
Show a | b.\n\
Show a ^ b.\n\
Show word8(1) shifted left by 7.\n\
Show word8(171) shifted left by 4.\n\
Show word8(128) shifted right by 7.\n\
Show rotl(word8(129), 1).\n\
Show rotr(word8(3), 1).\n";
const BITS_8_OUT: &str = "48\n252\n204\n128\n176\n1\n3\n129";

const INT_VIEW: &str = "## Main\n\
Show intOfWord16(word16(65535)) + 1.\n\
Show intOfWord8(word8(255)) + 1.\n\
Show word16(5) equals word16(65541).\n\
Show word8(3) is less than word8(4).\n\
Show word16(65535) is greater than word16(1).\n";
const INT_VIEW_OUT: &str = "65536\n256\ntrue\ntrue\ntrue";

const SEQ_16: &str = "## Main\n\
Let mutable s be a new Seq of Word16.\n\
Push word16(65535) to s.\n\
Push word16(3) to s.\n\
Push word16(40000) to s.\n\
Let mutable total be word16(0).\n\
Repeat for x in s:\n\
\x20   Set total to total + x.\n\
Show total.\n\
Show item 1 of s + item 2 of s.\n\
Set item 3 of s to item 3 of s * word16(2).\n\
Show s.\n\
Show length of s.\n";
const SEQ_16_OUT: &str = "40002\n2\n[65535, 3, 14464]\n3";

const FUNCTIONS: &str = "## To mix (a: Word16) and (b: Word16) -> Word16:\n\
\x20   Return (a + b) * word16(3).\n\
\n\
## To squash (x: Word8) -> Word8:\n\
\x20   Return x * x + word8(1).\n\
\n\
## Main\n\
Show mix(word16(65000), word16(1000)).\n\
Show squash(word8(20)).\n\
Let mutable acc be word8(0).\n\
Repeat for i from 1 to 100:\n\
\x20   Set acc to acc + word8(i).\n\
Show acc.\n";
const FUNCTIONS_OUT: &str = "1392\n145\n186";

const CORPUS: &[(&str, &str, &str)] = &[
    ("ring_16", RING_16, RING_16_OUT),
    ("ring_8", RING_8, RING_8_OUT),
    ("bits_16", BITS_16, BITS_16_OUT),
    ("bits_8", BITS_8, BITS_8_OUT),
    ("int_view", INT_VIEW, INT_VIEW_OUT),
    ("seq_16", SEQ_16, SEQ_16_OUT),
    ("functions", FUNCTIONS, FUNCTIONS_OUT),
];

#[test]
fn narrow_words_are_rings_on_the_tree_walker_and_vm() {
    for (name, src, out) in CORPUS {
        let r = run_all_tiers(src);
        assert_eq!(
            (r.output.trim(), r.error.as_deref()),
            (*out, None),
            "{name}\n{src}"
        );
    }
}

#[test]
fn every_word_width_mismatch_is_a_type_error() {
    for (a, b, wa, wb) in [
        ("word8(1)", "word16(1)", 8, 16),
        ("word16(1)", "word32(1)", 16, 32),
        ("word8(1)", "word64(1)", 8, 64),
        ("word16(1)", "word64(1)", 16, 64),
    ] {
        let src = format!("## Main\nShow {a} ^ {b}.\n");
        let r = run_all_tiers(&src);
        assert_eq!(
            r.error.as_deref(),
            Some(format!("cannot combine Word{wa} and Word{wb} — width mismatch").as_str()),
            "{src}"
        );
    }
}

#[test]
fn a_narrow_word_prints_like_its_value_everywhere() {
    assert_all_tiers(
        "## Main\nLet w be word16(513).\nShow \"w = {w}\".\nShow format(word8(7)).\nShow [word8(1), word8(2)].\n",
        "w = 513\n7\n[1, 2]",
    );
}

#[test]
fn narrow_words_are_rings_on_the_compiled_tier() {
    for (name, src, out) in CORPUS {
        eprintln!("compiled: {name}");
        assert_compiled_equals_interpreted_eq(src, out);
    }
}

/// Hot loops (well past the forge's tiering threshold) that compute in the narrow rings: the JIT must
/// never wrap a `Word16`/`Word8` at the 32-bit lane's modulus — the region either computes the narrow
/// ring exactly or stays on the VM.
const HOT_NARROW: &str = "## Main\n\
Let mutable acc be word16(0).\n\
Let mutable small be word8(0).\n\
Let mutable seen be 0.\n\
Let mutable i be 0.\n\
While i is less than 5000:\n\
\x20   Set acc to acc + word16(i * 37).\n\
\x20   Set small to small * word8(3) + word8(i).\n\
\x20   Set seen to seen + intOfWord16(word16(i * 977)).\n\
\x20   Set i to i + 1.\n\
Show acc.\n\
Show small.\n\
Show seen.\n";

fn hot_narrow_expected() -> String {
    let (mut acc, mut small, mut seen) = (0u16, 0u8, 0i64);
    for i in 0..5000i64 {
        acc = acc.wrapping_add((i * 37) as u16);
        small = small.wrapping_mul(3).wrapping_add(i as u8);
        seen += ((i * 977) as u16) as i64;
    }
    format!("{acc}\n{small}\n{seen}")
}

#[test]
fn narrow_words_under_the_forge_are_the_vms() {
    use logicaffeine_compile::compile::vm_outcome_with_args;
    use logicaffeine_compile::vm::NativeTier;
    let expected = hot_narrow_expected();
    let plain = run_all_tiers(HOT_NARROW);
    assert_eq!(
        (plain.output.trim(), plain.error.as_deref()),
        (expected.as_str(), None),
        "VM/tree-walker"
    );
    let tier = logicaffeine_jit::ForgeTier::new();
    let forged = vm_outcome_with_args(HOT_NARROW, &[], Some(&tier as &dyn NativeTier));
    assert_eq!(
        (forged.output.trim(), forged.error.as_deref()),
        (expected.as_str(), None),
        "forge JIT"
    );
}

#[test]
fn narrow_words_in_a_hot_loop_on_the_compiled_tier() {
    assert_compiled_equals_interpreted_eq(HOT_NARROW, &hot_narrow_expected());
}
