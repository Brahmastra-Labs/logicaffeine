//! The C runtime's `lbig_to_f64` is held to `logicaffeine_base::BigInt::to_f64`, the one definition of
//! how an integer becomes a Real: the nearest binary64, ties to even, infinite once the rounding reaches
//! 2^1024.
//!
//! The runtime's integer is base-10^9 limbs, so a conversion that rounds once per limb misses the nearest
//! float on a different set of values than the base-2^64 one, and a widened argument or `n + 0.5` then
//! prints differently on C than on every other tier. The harness links the runtime exactly as the C
//! backend emits it and reads `lbig_to_f64`'s bits for each integer of the corpus: random magnitudes of
//! 64 to 4096 bits of both signs, exact halfway cases and their neighbours, and the overflow boundary.

mod common;

use logicaffeine_base::BigInt;
use std::io::Write;
use std::process::{Command, Stdio};

fn xorshift(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn pow2(k: u64) -> BigInt {
    BigInt::from_u64(1).shl_bits(k)
}

/// Every integer the differential converts.
fn corpus() -> Vec<BigInt> {
    let mut out = Vec::new();
    let mut st = 0x5DEE_CE66_D1CE_4E5Bu64;
    for round in 0..4_000u64 {
        let bits = 64 + xorshift(&mut st) % (4096 - 64 + 1);
        let limbs = ((bits + 63) / 64) as usize;
        let mut bytes = Vec::with_capacity(limbs * 8);
        for _ in 0..limbs {
            bytes.extend_from_slice(&xorshift(&mut st).to_le_bytes());
        }
        let magnitude = BigInt::from_le_bytes(false, &bytes)
            .shr_bits(limbs as u64 * 64 - bits)
            .expect("non-negative");
        out.push(if round & 1 == 1 {
            magnitude.negated()
        } else {
            magnitude
        });
    }
    for _ in 0..1_500 {
        let significand = (xorshift(&mut st) >> 11) | (1u64 << 52);
        let shift = 1 + xorshift(&mut st) % 960;
        let halfway = BigInt::from_u64(significand)
            .shl_bits(shift + 1)
            .add(&pow2(shift));
        for n in [
            halfway.clone(),
            halfway.add(&BigInt::from_u64(1)),
            halfway.sub(&BigInt::from_u64(1)),
        ] {
            out.push(n.negated());
            out.push(n);
        }
    }
    let halfway_to_infinity = pow2(1024).sub(&pow2(970));
    for n in [
        halfway_to_infinity.clone(),
        halfway_to_infinity.sub(&BigInt::from_u64(1)),
        pow2(4096),
        BigInt::from_u64((1u64 << 53) + 1)
            .shl_bits(64)
            .add(&BigInt::from_u64(1)),
        BigInt::from_u64((1u64 << 53) + 1).shl_bits(64),
        BigInt::from_u64((1u64 << 53) + 3).shl_bits(64),
        BigInt::from_u64(u64::MAX),
        BigInt::from_i64(i64::MIN),
        BigInt::from_i64(0),
        BigInt::from_i64(1),
    ] {
        out.push(n.negated());
        out.push(n);
    }
    out
}

/// The harness: the program's `main` renamed away, and a `main` that reads `<sign> <decimal>` lines,
/// builds each integer in the runtime's own representation and prints `lbig_to_f64`'s bits.
const HARNESS: &str = r#"
int main(void) {
    static char line[1 << 14];
    while (fgets(line, sizeof line, stdin)) {
        char *digits = line + 2;
        size_t n = strcspn(digits, "\r\n");
        digits[n] = 0;
        LBig b = lbig_make(n / 9 + 2);
        size_t len = 0;
        for (size_t end = n; end > 0; ) {
            size_t start = end >= 9 ? end - 9 : 0;
            uint32_t limb = 0;
            for (size_t i = start; i < end; i++) limb = limb * 10u + (uint32_t)(digits[i] - '0');
            b.limb[len++] = limb;
            end = start;
        }
        b.len = len;
        lbig_trim(&b);
        if (b.len) b.sign = line[0] == '-' ? -1 : 1;
        double d = lbig_to_f64(&b);
        uint64_t bits;
        memcpy(&bits, &d, sizeof bits);
        printf("%016llx\n", (unsigned long long)bits);
        lbig_free(&b);
    }
    return 0;
}
"#;

#[test]
fn c_lbig_to_f64_is_bigint_to_f64_on_every_integer_of_the_corpus() {
    if !common::require_tool("gcc") {
        return;
    }
    let program = logicaffeine_compile::compile::compile_to_c("## Main\nShow 1.\n")
        .expect("the C backend lowers a one-line program");
    let renamed = program.replacen("int main(", "int logos_program_main(", 1);
    assert_ne!(renamed, program, "the emitted C defines `int main(`");
    let dir = tempfile::tempdir().expect("temp dir");
    let (src, bin) = (dir.path().join("harness.c"), dir.path().join("harness"));
    std::fs::write(&src, format!("{renamed}\n{HARNESS}")).expect("write the harness");
    let gcc = Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&src)
        .arg("-lm")
        .output()
        .expect("gcc runs");
    assert!(
        gcc.status.success(),
        "the harness builds:\n{}",
        String::from_utf8_lossy(&gcc.stderr)
    );

    let corpus = corpus();
    let mut input = String::new();
    for n in &corpus {
        let text = n.to_string();
        match text.strip_prefix('-') {
            Some(digits) => input.push_str(&format!("- {digits}\n")),
            None => input.push_str(&format!("+ {text}\n")),
        }
    }
    let mut child = Command::new(&bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the harness runs");
    let mut stdin = child.stdin.take().expect("piped stdin");
    let writer =
        std::thread::spawn(move || stdin.write_all(input.as_bytes()).expect("feed the harness"));
    let out = child.wait_with_output().expect("the harness finishes");
    writer.join().expect("the feeder");
    assert!(out.status.success(), "the harness exits cleanly");
    let got: Vec<&str> = std::str::from_utf8(&out.stdout)
        .expect("hex lines")
        .lines()
        .collect();
    assert_eq!(got.len(), corpus.len(), "one line per integer");

    let wrong: Vec<String> = corpus
        .iter()
        .zip(&got)
        .filter_map(|(n, c)| {
            let want = format!("{:016x}", n.to_f64().to_bits());
            (want != *c).then(|| format!("{n}: C {c}, BigInt::to_f64 {want}"))
        })
        .collect();
    assert!(
        wrong.is_empty(),
        "lbig_to_f64 differs from BigInt::to_f64 on {} of {} integers; the first five:\n{}",
        wrong.len(),
        corpus.len(),
        wrong.iter().take(5).cloned().collect::<Vec<_>>().join("\n")
    );
}
