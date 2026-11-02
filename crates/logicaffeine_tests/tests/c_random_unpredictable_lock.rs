//! `randomInt` and `randomFloat` are what a program makes its secrets from: `randomSecret` keys every
//! session cookie a server signs, and `pgScramClientNonce` draws the SCRAM handshake's nonce. On the C
//! tiers both come from the runtime's own generator, so that generator has to be one whose outputs say
//! nothing about the outputs after them. A generator whose whole state is one word it mixes out
//! invertibly (SplitMix64, xorshift, an LCG) gives itself away in a single full-width draw: invert the
//! mix, step the state, and every later draw — the next session secret — is known.
//!
//! A full-width draw is `randomInt(-(2^63 - 1), 2^63 - 1)`: its span is `2^64 - 1`, so the program
//! shows `lo + r` for the generator's raw word `r`, and the test reads `r` back exactly.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::process::Command;

use logicaffeine_compile::compile::{compile_to_c, compile_to_c_via_bytecode};

const DRAWS: usize = 16;
const LO: i64 = -9_223_372_036_854_775_807;

fn full_width_draws() -> String {
    format!(
        "## Main\nRepeat for i from 1 to {DRAWS}:\n    Show randomInt({LO}, 9223372036854775807).\n"
    )
}

/// `c_code` built as a release C build builds it and run once: the raw words of its draws, in order.
fn raw_words(lowering: &str, c_code: &str) -> Vec<u64> {
    let dir = tempfile::Builder::new()
        .prefix(&format!("logos_c_random_{lowering}_"))
        .tempdir()
        .expect("a run directory");
    let src = dir.path().join("prog.c");
    let bin = dir.path().join("prog");
    std::fs::write(&src, c_code).expect("write the C");
    let built = common::build_c_release(&src, &bin).expect("the C compiler starts");
    assert!(
        built.status.success(),
        "{lowering}: the emitted C builds\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let run = Command::new(&bin).output().expect("the program starts");
    assert!(
        run.status.success(),
        "{lowering}: the program runs\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let words: Vec<u64> = String::from_utf8_lossy(&run.stdout)
        .lines()
        .map(|line| {
            let shown: i128 = line.trim().parse().expect("a draw is an Int");
            u64::try_from(shown - LO as i128).expect("a draw lies in its range")
        })
        .collect();
    assert_eq!(words.len(), DRAWS, "{lowering}: every draw is shown");
    words
}

const GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
const M1: u64 = 0xBF58_476D_1CE4_E5B9;
const M2: u64 = 0x94D0_49BB_1331_11EB;

fn splitmix_mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(M1);
    z = (z ^ (z >> 27)).wrapping_mul(M2);
    z ^ (z >> 31)
}

/// The inverse of `x ↦ x * a` modulo 2^64, for odd `a` (Newton's iteration doubles the correct bits).
fn inverse(a: u64) -> u64 {
    let mut x = a;
    for _ in 0..6 {
        x = x.wrapping_mul(2u64.wrapping_sub(a.wrapping_mul(x)));
    }
    x
}

/// The inverse of `x ↦ x ^ (x >> s)`.
fn unshift(y: u64, s: u32) -> u64 {
    let mut x = y;
    let mut k = s;
    while k < 64 {
        x = y ^ (x >> s);
        k += s;
    }
    x
}

fn splitmix_unmix(z: u64) -> u64 {
    let z = unshift(z, 31).wrapping_mul(inverse(M2));
    let z = unshift(z, 27).wrapping_mul(inverse(M1));
    unshift(z, 30)
}

/// How many draws the SplitMix64 relation predicts from the draw before.
fn splitmix_predicted(words: &[u64]) -> usize {
    words
        .windows(2)
        .filter(|w| splitmix_mix(splitmix_unmix(w[0]).wrapping_add(GAMMA)) == w[1])
        .count()
}

#[test]
fn the_splitmix_inversion_this_suite_tests_with_is_exact() {
    let mut state = 0x0123_4567_89AB_CDEFu64;
    let mut words = Vec::new();
    for _ in 0..64 {
        state = state.wrapping_add(GAMMA);
        words.push(splitmix_mix(state));
        assert_eq!(splitmix_unmix(*words.last().unwrap()), state);
    }
    assert_eq!(
        splitmix_predicted(&words),
        63,
        "SplitMix64 is predicted draw after draw"
    );
    for a in [M1, M2, GAMMA, 1, u64::MAX] {
        assert_eq!(a.wrapping_mul(inverse(a)), 1);
    }
}

#[test]
fn an_ast_c_programs_draws_do_not_predict_the_next() {
    let c = compile_to_c(&full_width_draws()).expect("the program compiles to C");
    let words = raw_words("ast", &c);
    assert_eq!(
        splitmix_predicted(&words),
        0,
        "a draw from the C tier gives away the next one: {words:x?}"
    );
}

#[test]
fn a_bytecode_c_programs_draws_do_not_predict_the_next() {
    let c = compile_to_c_via_bytecode(&full_width_draws()).expect("the program compiles to C");
    let words = raw_words("bytecode", &c);
    assert_eq!(
        splitmix_predicted(&words),
        0,
        "a draw from the C tier gives away the next one: {words:x?}"
    );
}

/// The runtime's generator, keyed with `key` instead of the system's entropy, run in a harness that shows its
/// first `draws` words and then, three draws later, the words of its pool it has handed out.
fn generator_harness(key: &[u8; 32], draws: usize) -> (Vec<u64>, Vec<u64>) {
    generator_harness_as(Language::C, key, draws)
}

/// The language a harness is built as: the C tiers build the runtime as C, and the concurrency tier's
/// walker builds it into C++.
#[derive(Clone, Copy, Debug)]
enum Language {
    C,
    Cpp,
}

/// [`generator_harness`], built as `language`.
fn generator_harness_as(language: Language, key: &[u8; 32], draws: usize) -> (Vec<u64>, Vec<u64>) {
    let random_c = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../logicaffeine_compile/src/codegen_c/random.c"),
    )
    .expect("the C tier's generator source")
    .replace("@RANDOM_BOUNDS@", "\"%lld %lld\"");
    let words: Vec<String> = key
        .chunks(4)
        .map(|w| format!("{}u", u32::from_le_bytes(w.try_into().unwrap())))
        .collect();
    let harness = format!(
        "#include <stdio.h>\n#include <stdlib.h>\n#include <stdint.h>\n#include <string.h>\n\
         static void logos_fatal(const char *m) {{ fprintf(stderr, \"%s\\n\", m); exit(1); }}\n\
         static void logos_fatalf(const char *f, long long a, long long b) {{ fprintf(stderr, f, a, b); exit(1); }}\n\
         {random_c}\n\
         int main(void) {{\n\
             const uint32_t key[8] = {{{key}}};\n\
             memcpy(lc_rng_key, key, sizeof key);\n\
             lc_rng_keyed = 1;\n\
             for (int i = 0; i < {draws}; i++) printf(\"%llu\\n\", (unsigned long long)lc_rng_next());\n\
             for (int i = 0; i < 3; i++) lc_rng_next();\n\
             for (int i = lc_rng_left; i < LC_RNG_POOL; i++) printf(\"spent %llu\\n\", (unsigned long long)lc_rng_pool[i]);\n\
             return 0;\n\
         }}\n",
        key = words.join(", ")
    );
    let dir = tempfile::Builder::new()
        .prefix("logos_c_random_harness_")
        .tempdir()
        .expect("a run directory");
    let src = dir.path().join(match language {
        Language::C => "harness.c",
        Language::Cpp => "harness.cpp",
    });
    let bin = dir.path().join("harness");
    std::fs::write(&src, harness).expect("write the harness");
    let built = match language {
        Language::C => common::build_c_release(&src, &bin),
        Language::Cpp => Command::new(std::env::var("CXX").unwrap_or_else(|_| "c++".to_string()))
            .args(["-O2", "-Wall", "-o"])
            .arg(&bin)
            .arg(&src)
            .output(),
    }
    .expect("the compiler starts");
    assert!(
        built.status.success(),
        "the generator harness builds as {language:?}\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let run = Command::new(&bin).output().expect("the harness starts");
    assert!(run.status.success(), "the harness runs");
    let (mut shown, mut spent) = (Vec::new(), Vec::new());
    for line in String::from_utf8_lossy(&run.stdout).lines() {
        match line.strip_prefix("spent ") {
            Some(w) => spent.push(w.parse().unwrap()),
            None => shown.push(line.parse().unwrap()),
        }
    }
    (shown, spent)
}

/// Fast key erasure over RFC 8439's block function, from the system's ChaCha20: each refill is four blocks
/// (counter 0 to 3, nonce zero) under the current key; the first 32 bytes key the next refill and the other
/// 224 are the 28 words handed out, last first.
fn fast_key_erasure(mut key: [u8; 32], draws: usize) -> Vec<u64> {
    let mut out = Vec::new();
    while out.len() < draws {
        let stream = logicaffeine_system::aead::chacha20_xor(&key, &[0; 12], 0, &[0; 256]);
        key.copy_from_slice(&stream[..32]);
        for i in (0..28).rev() {
            out.push(u64::from_le_bytes(
                stream[32 + 8 * i..40 + 8 * i].try_into().unwrap(),
            ));
        }
    }
    out.truncate(draws);
    out
}

#[test]
fn the_generator_builds_as_cpp_and_draws_what_it_draws_as_c() {
    let key: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(29) ^ 0x3C);
    let (as_c, _) = generator_harness_as(Language::C, &key, 60);
    let (as_cpp, spent) = generator_harness_as(Language::Cpp, &key, 60);
    assert_eq!(as_cpp, as_c, "one key draws one stream in either language");
    assert_eq!(as_cpp, fast_key_erasure(key, 60));
    assert!(spent.iter().all(|&w| w == 0));
}

#[test]
fn the_c_generator_is_chacha20_with_fast_key_erasure() {
    for key in [
        std::array::from_fn(|i| i as u8),
        [0u8; 32],
        [0xFF; 32],
        std::array::from_fn(|i| (i as u8).wrapping_mul(37) ^ 0xA5),
    ] {
        let (shown, spent) = generator_harness(&key, 100);
        assert_eq!(shown, fast_key_erasure(key, 100), "key {key:02x?}");
        assert!(!spent.is_empty(), "the harness reads the words handed out");
        assert!(
            spent.iter().all(|&w| w == 0),
            "a word handed out stays erased in the pool: {spent:x?}"
        );
    }
}

/// A program's own names spelled as the generator's file-scope C names: each is mangled, so the program
/// builds, and its draws still come from the runtime's generator.
#[test]
fn a_program_spelled_like_the_generator_builds_and_draws() {
    let src = "## To lc_rng_refill (n: Int) -> Int:\n    Return n + 1.\n\n\
               ## To lc_rng_quarter (n: Int) -> Int:\n    Return n * 2.\n\n\
               ## To lc_rng_rotl (n: Int) -> Int:\n    Return n - 3.\n\n\
               ## To lc_rng_next -> Int:\n    Return 7.\n\n\
               ## Main\n\
               Let lc_rng_key be 5.\nLet lc_rng_pool be 6.\nLet lc_rng_left be 7.\nLet lc_rng_keyed be 8.\n\
               Show lc_rng_refill(lc_rng_key) + lc_rng_quarter(lc_rng_pool) + lc_rng_rotl(lc_rng_left) + lc_rng_keyed + lc_rng_next().\n\
               Let x be randomInt(1, 6).\nShow x is at least 1 and x is at most 6.\n";
    for (lowering, c) in [
        ("ast", compile_to_c(src).expect("the program compiles to C")),
        (
            "bytecode",
            compile_to_c_via_bytecode(src).expect("the program compiles to C"),
        ),
    ] {
        let dir = tempfile::Builder::new()
            .prefix(&format!("logos_c_random_names_{lowering}_"))
            .tempdir()
            .expect("a run directory");
        let (file, bin) = (dir.path().join("prog.c"), dir.path().join("prog"));
        std::fs::write(&file, &c).expect("write the C");
        let built = common::build_c_release(&file, &bin).expect("the C compiler starts");
        assert!(
            built.status.success(),
            "{lowering}: a program spelled like the generator builds\n{}",
            String::from_utf8_lossy(&built.stderr)
        );
        let run = Command::new(&bin).output().expect("the program starts");
        assert_eq!(
            String::from_utf8_lossy(&run.stdout),
            "37\ntrue\n",
            "{lowering}"
        );
    }
}

#[test]
fn two_runs_of_one_c_program_draw_different_streams() {
    let c = compile_to_c(&full_width_draws()).expect("the program compiles to C");
    let first = raw_words("ast_first", &c);
    let second = raw_words("ast_second", &c);
    assert_ne!(first, second, "each run keys its generator afresh");
}
