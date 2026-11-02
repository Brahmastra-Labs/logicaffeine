use super::c_runtime;

/// The C arbitrary-precision integer (LBig) must be correct: gcc-compile the runtime plus a main
/// that exercises pow (2^100), factorial-by-mul (25!), a carry across the base boundary, signed
/// subtraction, a negative-base power, and comparison — all decimal-exact. This is the Phase-2
/// foundation; the codegen integration (LOGOS `Int` → promoting LBig) builds on it.
#[test]
fn c_bignum_library_is_correct() {
    if std::process::Command::new("gcc")
        .arg("--version")
        .output()
        .map(|o| !o.status.success())
        .unwrap_or(true)
    {
        eprintln!("skip: gcc unavailable");
        return;
    }
    let main = r#"
int main(void) {
    LBig two = lbig_from_i64(2);
    LBig p = lbig_pow(&two, 100);           show_bignum(p);   /* 2^100 */
    LBig f = lbig_from_i64(1);
    for (int i = 2; i <= 25; i++) {
        LBig ii = lbig_from_i64(i), t = lbig_mul(&f, &ii);
        lbig_free(&f); lbig_free(&ii); f = t;
    }
    show_bignum(f);                          /* 25! */
    LBig a = lbig_from_i64(999999999), b = lbig_from_i64(2), s = lbig_add(&a, &b);
    show_bignum(s);                          /* 1000000001 (carry across base) */
    LBig x = lbig_from_i64(5), y = lbig_from_i64(8), d = lbig_sub(&x, &y);
    show_bignum(d);                          /* -3 */
    LBig nb = lbig_from_i64(-3), np = lbig_pow(&nb, 3);
    show_bignum(np);                         /* -27 */
    printf("%d\n", lbig_cmp(&p, &f));        /* 2^100 > 25! => 1 */
    return 0;
}
"#;
    let src = format!("{}\n{}", c_runtime(), main);
    let dir = std::env::temp_dir().join(format!("logos_bignum_ut_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cpath = dir.join("bn.c");
    let bin = dir.join("bn");
    std::fs::write(&cpath, &src).unwrap();
    let out = std::process::Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&cpath)
        .arg("-lm")
        .output()
        .expect("run gcc");
    assert!(
        out.status.success(),
        "the C bignum runtime must compile with gcc:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run = std::process::Command::new(&bin).output().expect("run");
    let got = String::from_utf8_lossy(&run.stdout);
    let want = "1267650600228229401496703205376\n\
                    15511210043330985984000000\n\
                    1000000001\n\
                    -3\n\
                    -27\n\
                    1\n";
    assert_eq!(got, want, "bignum output mismatch");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The C exact rational + complex tower (LRat / LComplex over LBig) must be byte-identical to the
/// interpreter: reduced fractions, complex `+`/`conj`/`÷` (fractional result), the modulus, and the
/// `3+4i` / `-i` / `5` Display forms. This is the substrate the `complex`/`decimal` C codegen uses.
#[test]
fn c_rational_complex_library_is_correct() {
    if std::process::Command::new("gcc")
        .arg("--version")
        .output()
        .map(|o| !o.status.success())
        .unwrap_or(true)
    {
        eprintln!("skip: gcc unavailable");
        return;
    }
    let main = r#"
int main(void) {
    LRat a = rat_from_i64(1), b = rat_from_i64(3), q = rat_div(&a, &b);
    printf("%s\n", rat_to_string(&q));                 /* 1/3 */
    LRat s1 = rat_div(&(LRat){lbig_from_i64(1),lbig_from_i64(1)}, &(LRat){lbig_from_i64(6),lbig_from_i64(1)});
    LRat sum = rat_add(&q, &s1);
    printf("%s\n", rat_to_string(&sum));               /* 1/3 + 1/6 = 1/2 */
    LComplex z34 = cplx_ints(3, 4);   show_complex(z34);           /* 3+4i */
    LComplex z12 = cplx_ints(1, 2), zadd = cplx_add(z12, z34); show_complex(zadd); /* 4+6i */
    LComplex cj = cplx_conj(z34);    show_complex(cj);            /* 3-4i */
    LComplex z50 = cplx_ints(5, 0), cj2 = cplx_conj(z50); show_complex(cj2); /* 5 */
    LComplex one = cplx_ints(1, 0), i = cplx_ints(0, 1), dv = cplx_div(one, i); show_complex(dv); /* -i */
    LComplex dv2 = cplx_div(z12, z34); show_complex(dv2);        /* 11/25+2/25i */
    printf("%.0f\n", cplx_abs_f64(z34));              /* |3+4i| = 5 */
    return 0;
}
"#;
    let src = format!("{}\n{}", c_runtime(), main);
    let dir = std::env::temp_dir().join(format!("logos_ratcplx_ut_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cpath = dir.join("rc.c");
    let bin = dir.join("rc");
    std::fs::write(&cpath, &src).unwrap();
    let out = std::process::Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&cpath)
        .arg("-lm")
        .output()
        .expect("run gcc");
    assert!(
        out.status.success(),
        "the C rational/complex runtime must compile:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run = std::process::Command::new(&bin).output().expect("run");
    let got = String::from_utf8_lossy(&run.stdout);
    let want = "1/3\n1/2\n3+4i\n4+6i\n3-4i\n5\n-i\n11/25+2/25i\n5\n";
    assert_eq!(got, want, "rational/complex output mismatch");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The C modular runtime (`LMod` over `LBig`) must be exact: `modular(v, n)` Euclidean-reduces on
/// construction, `+ − ×` wrap in the ring, `÷` multiplies by the extended-Euclidean inverse, and
/// `pow` is square-and-multiply — all byte-identical to the interpreter's `logicaffeine_base::Modular`.
/// A large (billion-scale) modulus stays exact. This is the substrate the `Modular` C codegen builds on.
#[test]
fn c_modular_library_is_correct() {
    if std::process::Command::new("gcc")
        .arg("--version")
        .output()
        .map(|o| !o.status.success())
        .unwrap_or(true)
    {
        eprintln!("skip: gcc unavailable");
        return;
    }
    let main = r#"
int main(void) {
    show_mod(mod_ints(10, 7));                                /* 3 (mod 7)  — reduce on construction */
    show_mod(mod_add(mod_ints(5, 7), mod_ints(4, 7)));        /* 2 (mod 7)  — 9 wraps */
    show_mod(mod_sub(mod_ints(2, 7), mod_ints(5, 7)));        /* 4 (mod 7)  — -3 Euclidean-reduces */
    show_mod(mod_mul(mod_ints(6, 7), mod_ints(6, 7)));        /* 1 (mod 7)  — 36 wraps */
    show_mod(mod_div(mod_ints(1, 7), mod_ints(3, 7)));        /* 5 (mod 7)  — 3^-1 = 5 */
    show_mod(mod_div(mod_ints(3, 7), mod_ints(5, 7)));        /* 2 (mod 7)  — 3 * 5^-1 */
    show_mod(mod_pow(mod_ints(2, 7), 5));                     /* 4 (mod 7)  — 2^5 = 32 */
    show_mod(mod_pow(mod_ints(3, 10), 4));                    /* 1 (mod 10) — 3^4 = 81 */
    show_mod(mod_ints(10, 1000000007));                       /* 10 (mod 1000000007) — large modulus */
    return 0;
}
"#;
    let src = format!("{}\n{}", c_runtime(), main);
    let dir = std::env::temp_dir().join(format!("logos_modular_ut_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cpath = dir.join("m.c");
    let bin = dir.join("m");
    std::fs::write(&cpath, &src).unwrap();
    let out = std::process::Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&cpath)
        .arg("-lm")
        .output()
        .expect("run gcc");
    assert!(
        out.status.success(),
        "the C modular runtime must compile:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run = std::process::Command::new(&bin).output().expect("run");
    let got = String::from_utf8_lossy(&run.stdout);
    let want = "3 (mod 7)\n2 (mod 7)\n4 (mod 7)\n1 (mod 7)\n5 (mod 7)\n2 (mod 7)\n4 (mod 7)\n1 (mod 10)\n10 (mod 1000000007)\n";
    assert_eq!(got, want, "modular runtime output mismatch");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The C UUID runtime (`LUuid` = 16 bytes) must be exact: canonical parse round-trips, nil/max and
/// the four RFC 9562 namespace constants render right, the version nibble reads, a 16-byte compare
/// powers `==`, `uuid_bytes`/`uuid_from_bytes` round-trip, and the `urn:uuid:`/`{…}`/32-hex input
/// forms all normalize to the same canonical string. This is the substrate the `Uuid` C codegen uses.
#[test]
fn c_uuid_library_is_correct() {
    if std::process::Command::new("gcc")
        .arg("--version")
        .output()
        .map(|o| !o.status.success())
        .unwrap_or(true)
    {
        eprintln!("skip: gcc unavailable");
        return;
    }
    let main = r#"
int main(void) {
    show_uuid(uuid_parse("550e8400-e29b-41d4-a716-446655440000"));
    show_uuid(uuid_nil());
    show_uuid(uuid_max());
    show_uuid(uuid_dns());
    show_uuid(uuid_url());
    show_uuid(uuid_oid());
    show_uuid(uuid_x500());
    printf("%lld\n", (long long) uuid_version(uuid_parse("550e8400-e29b-41d4-a716-446655440000")));
    printf("%d\n", uuid_eq(uuid_nil(), uuid_nil()));
    printf("%d\n", uuid_eq(uuid_nil(), uuid_max()));
    show_uuid(uuid_from_bytes(uuid_bytes(uuid_parse("550e8400-e29b-41d4-a716-446655440000"))));
    show_uuid(uuid_parse("urn:uuid:550e8400-e29b-41d4-a716-446655440000"));
    show_uuid(uuid_parse("{550e8400-e29b-41d4-a716-446655440000}"));
    show_uuid(uuid_parse("550e8400e29b41d4a716446655440000"));
    return 0;
}
"#;
    let src = format!("{}\n{}", c_runtime(), main);
    let dir = std::env::temp_dir().join(format!("logos_uuid_ut_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cpath = dir.join("u.c");
    let bin = dir.join("u");
    std::fs::write(&cpath, &src).unwrap();
    let out = std::process::Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&cpath)
        .arg("-lm")
        .output()
        .expect("run gcc");
    assert!(
        out.status.success(),
        "the C uuid runtime must compile:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run = std::process::Command::new(&bin).output().expect("run");
    let got = String::from_utf8_lossy(&run.stdout);
    let want = "550e8400-e29b-41d4-a716-446655440000\n00000000-0000-0000-0000-000000000000\nffffffff-ffff-ffff-ffff-ffffffffffff\n6ba7b810-9dad-11d1-80b4-00c04fd430c8\n6ba7b811-9dad-11d1-80b4-00c04fd430c8\n6ba7b812-9dad-11d1-80b4-00c04fd430c8\n6ba7b814-9dad-11d1-80b4-00c04fd430c8\n4\n1\n0\n550e8400-e29b-41d4-a716-446655440000\n550e8400-e29b-41d4-a716-446655440000\n550e8400-e29b-41d4-a716-446655440000\n550e8400-e29b-41d4-a716-446655440000\n";
    assert_eq!(got, want, "uuid runtime output mismatch");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The C scale-aware Decimal (`LDec`) must be exact: fixed-point construction from a string,
/// scale-preserving `+ − ×`, the Decimal→Rational flip on non-terminating `÷` (staying a fraction
/// through later arithmetic), a negative value, and banker's-rounding `rescale` (`0.125`→`0.12`,
/// `0.135`→`0.14`). This is the substrate the `decimal` C codegen and `Money` build on.
#[test]
fn c_decimal_library_is_correct() {
    if std::process::Command::new("gcc")
        .arg("--version")
        .output()
        .map(|o| !o.status.success())
        .unwrap_or(true)
    {
        eprintln!("skip: gcc unavailable");
        return;
    }
    let main = r#"
int main(void) {
    show_ldec(ldec_from_text("0.10"));
    show_ldec(ldec_add(ldec_from_text("0.5"), ldec_from_text("0.25")));
    show_ldec(ldec_mul(ldec_from_text("1.50"), ldec_from_i64(2)));
    show_ldec(ldec_add(ldec_from_i64(10), ldec_from_i64(2)));
    show_ldec(ldec_div(ldec_from_i64(1), ldec_from_i64(3)));
    show_ldec(ldec_add(ldec_div(ldec_from_i64(1), ldec_from_i64(3)), ldec_div(ldec_from_i64(1), ldec_from_i64(6))));
    show_ldec(ldec_from_text("-1.50"));
    show_ldec(ldec_from_text("100.00"));
    show_ldec(ldec_rescale(ldec_from_text("0.125"), 2));
    show_ldec(ldec_rescale(ldec_from_text("0.135"), 2));
    return 0;
}
"#;
    let src = format!("{}\n{}", c_runtime(), main);
    let dir = std::env::temp_dir().join(format!("logos_ldec_ut_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cpath = dir.join("d.c");
    let bin = dir.join("d");
    std::fs::write(&cpath, &src).unwrap();
    let out = std::process::Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&cpath)
        .arg("-lm")
        .output()
        .expect("run gcc");
    assert!(
        out.status.success(),
        "the C decimal runtime must compile:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run = std::process::Command::new(&bin).output().expect("run");
    let got = String::from_utf8_lossy(&run.stdout);
    let want = "0.10\n0.75\n3.00\n12\n1/3\n1/2\n-1.50\n100.00\n0.12\n0.14\n";
    assert_eq!(got, want, "decimal runtime output mismatch");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The C Money runtime (`LMoney` = `LDec` amount + ISO-4217 currency) must be exact: quantise int
/// and decimal amounts to the currency's minor unit (USD scale 2, JPY 0, BHD 3), same-currency
/// add/sub, integer scaling (`19.99 × 3 = 59.97`), a dimensionless ratio (`30/10 = 3`), and a
/// case-insensitive currency code (`"usd"` → `USD`).
#[test]
fn c_money_library_is_correct() {
    if std::process::Command::new("gcc")
        .arg("--version")
        .output()
        .map(|o| !o.status.success())
        .unwrap_or(true)
    {
        eprintln!("skip: gcc unavailable");
        return;
    }
    let main = r#"
int main(void) {
    show_money(money_of(ldec_from_i64(10), "USD"));
    show_money(money_of(ldec_from_i64(100), "JPY"));
    show_money(money_of(ldec_from_i64(5), "BHD"));
    show_money(money_of(ldec_from_text("19.99"), "USD"));
    show_money(money_add(money_of(ldec_from_text("0.10"), "USD"), money_of(ldec_from_text("0.20"), "USD")));
    show_money(money_sub(money_of(ldec_from_i64(30), "USD"), money_of(ldec_from_i64(10), "USD")));
    show_money(money_scale(money_of(ldec_from_text("19.99"), "USD"), 3));
    show_ldec(money_ratio(money_of(ldec_from_i64(30), "USD"), money_of(ldec_from_i64(10), "USD")));
    show_money(money_of(ldec_from_i64(10), "usd"));
    return 0;
}
"#;
    let src = format!("{}\n{}", c_runtime(), main);
    let dir = std::env::temp_dir().join(format!("logos_money_ut_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cpath = dir.join("m.c");
    let bin = dir.join("m");
    std::fs::write(&cpath, &src).unwrap();
    let out = std::process::Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&cpath)
        .arg("-lm")
        .output()
        .expect("run gcc");
    assert!(
        out.status.success(),
        "the C money runtime must compile:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run = std::process::Command::new(&bin).output().expect("run");
    let got = String::from_utf8_lossy(&run.stdout);
    let want =
        "10.00 USD\n100 JPY\n5.000 BHD\n19.99 USD\n0.30 USD\n20.00 USD\n59.97 USD\n3\n10.00 USD\n";
    assert_eq!(got, want, "money runtime output mismatch");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The C Quantity runtime (`LQuant` = exact-Rational SI magnitude + 10-axis dimension vector +
/// display unit) must be exact: construction shows the magnitude in its unit (`5 m`, `2 in`, `3 kg`),
/// same-dimension add/sub, `in`-conversion is exact rational (`5 m → 6250/381 ft`), the
/// dimension-combining `× ÷` render the dimension SIGNATURE (`6 L^2`, dimensionless `1`), and a
/// scalar `×` preserves the unit (`10 m`). Codegen resolves units → these scale/dimension constants.
#[test]
fn c_quantity_library_is_correct() {
    if std::process::Command::new("gcc")
        .arg("--version")
        .output()
        .map(|o| !o.status.success())
        .unwrap_or(true)
    {
        eprintln!("skip: gcc unavailable");
        return;
    }
    let main = r#"
int main(void) {
    const int L[10] = {1,0,0,0,0,0,0,0,0,0};
    const int M[10] = {0,1,0,0,0,0,0,0,0,0};
    const int ONE[10] = {1,1,1,1,1,1,1,1,1,1};
    LQuant m2 = quant_of(rat_ratio(2,1), rat_ratio(1,1), rat_ratio(0,1), L, ONE, "m");
    LQuant m3 = quant_of(rat_ratio(3,1), rat_ratio(1,1), rat_ratio(0,1), L, ONE, "m");
    LQuant m5 = quant_of(rat_ratio(5,1), rat_ratio(1,1), rat_ratio(0,1), L, ONE, "m");
    LQuant m100 = quant_of(rat_ratio(100,1), rat_ratio(1,1), rat_ratio(0,1), L, ONE, "m");
    LQuant m4 = quant_of(rat_ratio(4,1), rat_ratio(1,1), rat_ratio(0,1), L, ONE, "m");
    show_quant(m5);
    show_quant(quant_of(rat_ratio(2,1), rat_ratio(127,5000), rat_ratio(0,1), L, ONE, "in"));
    show_quant(quant_of(rat_ratio(3,1), rat_ratio(1,1), rat_ratio(0,1), M, ONE, "kg"));
    show_quant(quant_add(m2, m3));
    show_quant(quant_sub(m5, m2));
    show_quant(quant_convert(m5, rat_ratio(381,1250), rat_ratio(0,1), L, ONE, "ft"));
    show_quant(quant_mul(m2, m3));
    show_quant(quant_div(m100, m4));
    show_quant(quant_scale_mul(m5, rat_ratio(2,1)));
    return 0;
}
"#;
    let src = format!("{}\n{}", c_runtime(), main);
    let dir = std::env::temp_dir().join(format!("logos_quant_ut_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cpath = dir.join("q.c");
    let bin = dir.join("q");
    std::fs::write(&cpath, &src).unwrap();
    let out = std::process::Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&cpath)
        .arg("-lm")
        .output()
        .expect("run gcc");
    assert!(
        out.status.success(),
        "the C quantity runtime must compile:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run = std::process::Command::new(&bin).output().expect("run");
    let got = String::from_utf8_lossy(&run.stdout);
    let want = "5 m\n2 in\n3 kg\n5 m\n3 m\n6250/381 ft\n6 L^2\n25 1\n10 m\n";
    assert_eq!(got, want, "quantity runtime output mismatch");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The C tagged big integer (`LInt`, the `Int` mirror) must promote to `LBig` EXACTLY on overflow
/// and stay on the int64 fast path otherwise: `1e10 * 1e10` (overflows i64 → decimal-exact), a
/// following `+ 123`, small-stays-small (`6*7`), signed subtraction, factorial-by-mul (25!), a
/// bignum truncating division, and comparison. This is the substrate the `Int` C codegen builds on.
#[test]
fn c_tagged_bigint_library_is_correct() {
    if std::process::Command::new("gcc")
        .arg("--version")
        .output()
        .map(|o| !o.status.success())
        .unwrap_or(true)
    {
        eprintln!("skip: gcc unavailable");
        return;
    }
    let main = r#"
int main(void) {
    LInt a = lint_mul(lint_small(10000000000LL), lint_small(10000000000LL)); /* 1e20 */
    show_lint(lint_add(a, lint_small(123)));            /* 100000000000000000123 */
    show_lint(lint_mul(lint_small(6), lint_small(7)));  /* 42 (fast path) */
    show_lint(lint_sub(lint_small(5), lint_small(8)));  /* -3 */
    LInt f = lint_small(1);
    for (int i = 2; i <= 25; i++) f = lint_mul(f, lint_small(i));
    show_lint(f);                                       /* 25! */
    show_lint(lint_div(a, lint_small(1000000000LL)));   /* 1e20 / 1e9 = 100000000000 */
    printf("%d\n", lint_cmp(a, f));                     /* 1e20 > 25! => -1 */
    return 0;
}
"#;
    let src = format!("{}\n{}", c_runtime(), main);
    let dir = std::env::temp_dir().join(format!("logos_lint_ut_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cpath = dir.join("li.c");
    let bin = dir.join("li");
    std::fs::write(&cpath, &src).unwrap();
    let out = std::process::Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&cpath)
        .arg("-lm")
        .output()
        .expect("run gcc");
    assert!(
        out.status.success(),
        "the C tagged-bigint runtime must compile:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run = std::process::Command::new(&bin).output().expect("run");
    let got = String::from_utf8_lossy(&run.stdout);
    let want = "100000000000000000123\n42\n-3\n15511210043330985984000000\n100000000000\n-1\n";
    assert_eq!(got, want, "tagged-bigint output mismatch");
    let _ = std::fs::remove_dir_all(&dir);
}
