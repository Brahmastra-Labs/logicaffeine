//! The C backend (`largo emit c`) must auto-import stdlib `.lg` functions exactly like the Rust
//! backend, and must mangle LOGOS function names that collide with C library/builtin names
//! (`sin`, `cos`, `abs`, …) so the generated C actually links and compiles.
//!
//! Regression lock for the two bugs found while making LOGOS beat hand-tuned C through the C/gcc
//! backend: (1) `compile_to_c` skipped `apply_prelude` → stdlib calls with no body ("undefined
//! reference to binomial"); (2) splicing math.lg emitted `int64_t sin(...)` clashing with libm's
//! builtin `double sin(double)` ("conflicting types for 'sin'").

use logicaffeine_compile::compile::compile_to_c;

/// (1) A call to an auto-imported stdlib function emits the function BODY, not just the call.
#[test]
fn c_backend_autoimports_stdlib_body() {
    let c = compile_to_c("## Main\nShow gcd(12, 18).\n").expect("compiles to C");
    let defines_gcd = c.contains("gcd(int64_t") || c.contains("int64_t gcd(");
    assert!(
        defines_gcd,
        "C backend must EMIT the auto-imported stdlib gcd body, not just call it (undefined reference otherwise)"
    );
}

/// (2) A LOGOS function whose name collides with a libm builtin is mangled (`sin` → `logos_sin`)
/// at BOTH definition and call, so the C compiler sees no conflicting declaration. Uses a
/// reachable function (a user `sin`) — the pruner drops *unused* colliding functions entirely, so
/// the mangling only needs to hold for ones actually emitted.
#[test]
fn c_backend_mangles_libm_name_collisions() {
    let c = compile_to_c("## To sin (x: Int) -> Int:\n    Return x + 1.\n## Main\nShow sin(4).\n")
        .expect("compiles to C");
    assert!(
        c.contains("logos_sin"),
        "libm-colliding `sin` must be mangled to `logos_sin`"
    );
    // No bare `sin(` DEFINITION or CALL emitted (would clash with the compiler builtin).
    assert!(
        !c.contains(" sin(") && !c.contains("\nsin(") && !c.contains("t sin("),
        "no bare `sin(` may survive — it collides with the libm builtin"
    );
}

/// (3) End-to-end: the emitted C for a stdlib-using program actually compiles with gcc and runs
/// correctly. Skipped (not failed) when gcc is unavailable, so CI without a C toolchain stays green.
#[test]
fn c_backend_stdlib_program_gcc_compiles_and_runs() {
    let gcc = std::process::Command::new("gcc").arg("--version").output();
    if gcc.map(|o| !o.status.success()).unwrap_or(true) {
        eprintln!("skip: gcc unavailable");
        return;
    }
    let c = compile_to_c("## Main\nShow binomial(30, 5).\n").expect("compiles to C");
    let dir = std::env::temp_dir().join(format!("logos_c_prelude_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let src = dir.join("out.c");
    let bin = dir.join("out");
    std::fs::write(&src, &c).unwrap();
    let out = std::process::Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&src)
        .arg("-lm")
        .output()
        .expect("run gcc");
    assert!(
        out.status.success(),
        "emitted C must compile with gcc; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run = std::process::Command::new(&bin)
        .output()
        .expect("run binary");
    let got = String::from_utf8_lossy(&run.stdout);
    assert_eq!(got.trim(), "142506", "binomial(30,5) = 142506");
    let _ = std::fs::remove_dir_all(&dir);
}

/// (4) The breadth lock: a cross-domain battery of stdlib capabilities must compile with gcc AND
/// produce the right answer through the C backend. Covers text (text_bytes/text_from_bytes +
/// composition), iter over Seq literals (the materialize-in-any-position fix), Seq of Real
/// (Real→double typing), heap, sort predicates, bits, convert, collections, statistics, and a
/// float-returning builtin (sqrt typing). Skipped when gcc is unavailable.
#[test]
fn c_backend_stdlib_breadth_compiles_and_runs() {
    let gcc = std::process::Command::new("gcc").arg("--version").output();
    if gcc.map(|o| !o.status.success()).unwrap_or(true) {
        eprintln!("skip: gcc unavailable");
        return;
    }
    let cases: &[(&str, &str)] = &[
        // text: nested composition through text_bytes/text_from_bytes
        ("## Main\nShow upper(reverseText(\"hello\")).\n", "OLLEH"),
        (
            "## Main\nShow join(split(\"a,b,c\", \",\"), \"-\").\n",
            "a-b-c",
        ),
        ("## Main\nShow countSub(\"banana\", \"a\").\n", "3"),
        // iter over Seq literals (materialized in argument position)
        ("## Main\nShow sumSeq([10, 20, 30]).\n", "60"),
        (
            "## Main\nLet s be sortSeq([3, 1, 2]).\nShow item 1 of s.\n",
            "1",
        ),
        ("## Main\nShow maxSeq([3, 1, 9, 2]).\n", "9"),
        ("## Main\nShow bisectLeft([1, 3, 5], 4).\n", "3"),
        // heap
        (
            "## Main\nLet s be heapSortSeq([5, 3, 8, 1, 9]).\nShow item 2 of s.\n",
            "3",
        ),
        // Seq of Real (Real -> double)
        ("## Main\nShow meanReal([1.0, 2.0, 3.0]).\n", "2"),
        // statistics: OLS slope over two Seq-of-Real reductions
        (
            "## Main\nShow linRegSlope([0.0, 1.0, 2.0, 3.0], [1.0, 3.0, 5.0, 7.0]).\n",
            "2",
        ),
        // statistics: exclusive-method quantile cut point (sort + integer divmod + Real interpolation)
        (
            "## Main\nShow floor(quantile([1.0, 2.0, 3.0, 4.0], 1, 4) * 100.0 + 0.5).\n",
            "125",
        ),
        // float-returning builtin typing
        ("## Main\nShow sqrt(2.0).\n", "1.41421"),
        // special functions (erf via exp; remainder via floor + Int->Real parseFloat bridge)
        ("## Main\nShow fma(2.0, 3.0, 4.0).\n", "10"),
        ("## Main\nShow floor(erf(1.0) * 100.0).\n", "84"),
        ("## Main\nShow remainderReal(10.0, 4.0).\n", "2"),
        // strconv formatters + banker's rounding
        ("## Main\nShow appendInt(\"x=\", 42).\n", "x=42"),
        ("## Main\nShow appendBool(\"b:\", true).\n", "b:true"),
        ("## Main\nShow quoteRune(65).\n", "'A'"),
        ("## Main\nShow appendQuoteRune(\"c=\", 65).\n", "c='A'"),
        ("## Main\nShow roundEvenReal(2.5).\n", "2"),
        ("## Main\nShow isClose(1.0, 1.0000000001).\n", "true"),
        (
            "## Main\nShow floor(absReal(0.0 - 3.5) * 10.0 + 0.5).\n",
            "35",
        ),
        ("## Main\nShow ilogb(1024.0).\n", "10"),
        ("## Main\nShow floor(modfFrac(3.75) * 100.0 + 0.5).\n", "75"),
        ("## Main\nShow frexpExp(3.0).\n", "2"),
        (
            "## Main\nShow floor(frexpFrac(3.0) * 1000.0 + 0.5).\n",
            "750",
        ),
        ("## Main\nShow cmpInt(3, 5).\n", "-1"),
        ("## Main\nShow cmpAbsInt(0 - 5, 3).\n", "1"),
        ("## Main\nShow quoRemQ(0 - 7, 3).\n", "-2"),
        ("## Main\nShow divModM(0 - 7, 3).\n", "2"),
        // calendar: 6-arg + cross-module day arithmetic (daysBeforeYear/daysInMonth)
        ("## Main\nShow timegm(1970, 1, 2, 0, 0, 0).\n", "86400"),
        // math/big: mulRange compiles + runs on the C tier for int64-representable results
        // (the C backend is int64, NOT arbitrary-precision — 25! overflows there, so the
        // bignum guarantee is proven only on the 4 non-C tiers by parity_math_mulrange).
        ("## Main\nShow mulRange(3, 6).\n", "360"),
        // math/big number theory (int64-safe moduli → byte-identical on the C tier too)
        ("## Main\nShow jacobi(2, 7).\n", "1"),
        ("## Main\nShow isProbablePrime(97).\n", "true"),
        (
            "## Main\nShow (modSqrt(2, 7) * modSqrt(2, 7)) mod 7.\n",
            "2",
        ),
        // bits / convert / collections
        ("## Main\nShow popcount(255).\n", "8"),
        ("## Main\nShow toBase(255, 16).\n", "ff"),
        (
            "## Main\nLet c be countInts([1, 2, 2, 3]).\nShow counterGet(c, 2).\n",
            "2",
        ),
    ];
    let dir = std::env::temp_dir().join(format!("logos_c_breadth_{}", std::process::id()));
    for (i, (src, want)) in cases.iter().enumerate() {
        let c =
            compile_to_c(src).unwrap_or_else(|e| panic!("compile_to_c failed for {src:?}: {e:?}"));
        let _ = std::fs::create_dir_all(&dir);
        let cpath = dir.join(format!("c{i}.c"));
        let bin = dir.join(format!("b{i}"));
        std::fs::write(&cpath, &c).unwrap();
        let out = std::process::Command::new("gcc")
            .args(["-O2", "-o"])
            .arg(&bin)
            .arg(&cpath)
            .arg("-lm")
            .output()
            .expect("run gcc");
        assert!(
            out.status.success(),
            "gcc failed for {src:?}:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let run = std::process::Command::new(&bin).output().expect("run");
        let got = String::from_utf8_lossy(&run.stdout);
        assert!(
            got.trim().starts_with(want),
            "{src:?}: got {:?}, want prefix {want:?}",
            got.trim()
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// (5) Closures / higher-order functions through C: a lambda argument is lifted to a top-level
/// `static` C function and passed as a function pointer to the `fn(...)`-typed stdlib parameter.
/// Covers map/filter/reduce/any/all. Skipped when gcc is unavailable.
#[test]
fn c_backend_closures_compile_and_run() {
    let gcc = std::process::Command::new("gcc").arg("--version").output();
    if gcc.map(|o| !o.status.success()).unwrap_or(true) {
        eprintln!("skip: gcc unavailable");
        return;
    }
    let cases: &[(&str, &str)] = &[
        ("## Main\nLet r be mapSeq((n: Int) -> n * 2, [1, 2, 3]).\nShow item 3 of r.\n", "6"),
        ("## Main\nLet r be filterSeq((n: Int) -> n is greater than 2, [1, 2, 3, 4]).\nShow length of r.\n", "2"),
        ("## Main\nShow reduceSeq((a: Int, x: Int) -> a + x, 0, [1, 2, 3, 4]).\n", "10"),
        ("## Main\nShow anySeq((n: Int) -> n is greater than 3, [1, 2, 3, 4]).\n", "true"),
        ("## Main\nShow allSeq((n: Int) -> n is greater than 0, [1, 2, 3]).\n", "true"),
    ];
    let dir = std::env::temp_dir().join(format!("logos_c_clos_{}", std::process::id()));
    for (i, (src, want)) in cases.iter().enumerate() {
        let c =
            compile_to_c(src).unwrap_or_else(|e| panic!("compile_to_c failed for {src:?}: {e:?}"));
        let _ = std::fs::create_dir_all(&dir);
        let cpath = dir.join(format!("c{i}.c"));
        let bin = dir.join(format!("b{i}"));
        std::fs::write(&cpath, &c).unwrap();
        let out = std::process::Command::new("gcc")
            .args(["-O2", "-o"])
            .arg(&bin)
            .arg(&cpath)
            .arg("-lm")
            .output()
            .expect("run gcc");
        assert!(
            out.status.success(),
            "closure gcc failed for {src:?}:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let run = std::process::Command::new(&bin).output().expect("run");
        assert_eq!(
            String::from_utf8_lossy(&run.stdout).trim(),
            *want,
            "{src:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// (6) Seq of struct through C: `zipSeq`/`enumerateSeq` return `Seq of IntPair`, which lowers to a
/// generated per-struct `Seq_IntPair` (new/push/get/len). Indexing yields the struct so `p's first`
/// reads a real field. The last gap to the whole stdlib compiling through C. Skipped without gcc.
#[test]
fn c_backend_seq_of_struct_compiles_and_runs() {
    let gcc = std::process::Command::new("gcc").arg("--version").output();
    if gcc.map(|o| !o.status.success()).unwrap_or(true) {
        eprintln!("skip: gcc unavailable");
        return;
    }
    let cases: &[(&str, &str)] = &[
        ("## Main\nLet z be zipSeq([1, 2], [4, 5]).\nLet p be item 1 of z.\nShow p's second.\n", "4"),
        ("## Main\nLet e be enumerateSeq([10, 20, 30]).\nLet p be item 2 of e.\nShow p's first.\n", "2"),
        ("## Main\nLet e be enumerateSeq([10, 20, 30]).\nShow length of e.\n", "3"),
    ];
    let dir = std::env::temp_dir().join(format!("logos_c_struct_{}", std::process::id()));
    for (i, (src, want)) in cases.iter().enumerate() {
        let c =
            compile_to_c(src).unwrap_or_else(|e| panic!("compile_to_c failed for {src:?}: {e:?}"));
        let _ = std::fs::create_dir_all(&dir);
        let cpath = dir.join(format!("c{i}.c"));
        let bin = dir.join(format!("b{i}"));
        std::fs::write(&cpath, &c).unwrap();
        let out = std::process::Command::new("gcc")
            .args(["-O2", "-o"])
            .arg(&bin)
            .arg(&cpath)
            .arg("-lm")
            .output()
            .expect("run gcc");
        assert!(
            out.status.success(),
            "seq-of-struct gcc failed for {src:?}:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let run = std::process::Command::new(&bin).output().expect("run");
        assert_eq!(
            String::from_utf8_lossy(&run.stdout).trim(),
            *want,
            "{src:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// (7) Stateful buffers + the extended regex engine through C. The buffer projections thread the
/// `StringBuilder`/`ByteReader` structs functionally (builder.lg); the regex ops build on a
/// match-length engine (`reMatchEnd`) that returns `Seq of Text` and recurses mutually. Both must
/// compile with gcc and produce byte-identical answers — locking the `aot_c` tier claim for the
/// builder + regex capabilities landed on the existing stateful-object pattern. Skipped without gcc.
#[test]
fn c_backend_buffers_and_regex_compile_and_run() {
    let gcc = std::process::Command::new("gcc").arg("--version").output();
    if gcc.map(|o| !o.status.success()).unwrap_or(true) {
        eprintln!("skip: gcc unavailable");
        return;
    }
    let cases: &[(&str, &str)] = &[
        // buffer write side: WriteByte appends chr(c); Bytes materializes the byte view
        ("## Main\nShow builderString(builderWriteByte(builderWriteString(newBuilder(), \"A\"), 66)).\n", "AB"),
        ("## Main\nShow builderBytes(builderWriteString(newBuilder(), \"AB\")).\n", "[65, 66]"),
        ("## Main\nShow builderString(builderTruncate(builderWriteString(newBuilder(), \"Hello\"), 3)).\n", "Hel"),
        // reader side: size / unread / next-n over the threaded cursor
        ("## Main\nShow readerSize(newReader([65, 66, 67])).\n", "3"),
        ("## Main\nLet r be readerAdvance(readerAdvance(newReader([65, 66, 67]))).\nShow readerReadByte(readerUnreadByte(r)).\n", "66"),
        ("## Main\nShow readerNext(newReader([65, 66, 67]), 2).\n", "[65, 66]"),
        // regex composition ops on the match-length engine
        ("## Main\nShow regexMatch(\"al*o\", \"aaaloha\").\n", "true"),
        ("## Main\nShow regexFindAll(\"a\", \"banana\").\n", "[2, 4, 6]"),
        ("## Main\nShow regexReplaceAll(\"an\", \"banana\", \"X\").\n", "bXXa"),
        ("## Main\nLet ps be regexSplit(\",\", \"a,b,c\").\nShow item 2 of ps.\n", "b"),
    ];
    let dir = std::env::temp_dir().join(format!("logos_c_bufre_{}", std::process::id()));
    for (i, (src, want)) in cases.iter().enumerate() {
        let c =
            compile_to_c(src).unwrap_or_else(|e| panic!("compile_to_c failed for {src:?}: {e:?}"));
        let _ = std::fs::create_dir_all(&dir);
        let cpath = dir.join(format!("c{i}.c"));
        let bin = dir.join(format!("b{i}"));
        std::fs::write(&cpath, &c).unwrap();
        let out = std::process::Command::new("gcc")
            .args(["-O2", "-o"])
            .arg(&bin)
            .arg(&cpath)
            .arg("-lm")
            .output()
            .expect("run gcc");
        assert!(
            out.status.success(),
            "buffer/regex gcc failed for {src:?}:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let run = std::process::Command::new(&bin).output().expect("run");
        assert_eq!(
            String::from_utf8_lossy(&run.stdout).trim(),
            *want,
            "{src:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// (8) The bytes-as-ASCII-text wave through C: hex codec + byte-sequence predicates + case
/// transforms (bytes.lg), all pure `Seq of Int` / `Text` over `text_bytes`/`text_from_bytes`.
/// Must compile with gcc and stay byte-identical — locks the `aot_c` tier for the hex + bytes
/// classification/case capabilities. Skipped without gcc.
#[test]
fn c_backend_bytes_ascii_compile_and_run() {
    let gcc = std::process::Command::new("gcc").arg("--version").output();
    if gcc.map(|o| !o.status.success()).unwrap_or(true) {
        eprintln!("skip: gcc unavailable");
        return;
    }
    let cases: &[(&str, &str)] = &[
        ("## Main\nShow bytesToHex([255, 0, 171]).\n", "ff00ab"),
        (
            "## Main\nShow bytesFromHex(\"48656c6c6f\").\n",
            "[72, 101, 108, 108, 111]",
        ),
        ("## Main\nShow bytesIsDigit([49, 50, 51]).\n", "true"),
        ("## Main\nShow bytesIsUpper([72, 105]).\n", "false"),
        ("## Main\nShow bytesCapitalize([104, 105]).\n", "[72, 105]"),
        ("## Main\nShow bytesSwapCase([72, 105]).\n", "[104, 73]"),
        (
            "## Main\nShow bytesTitle([104, 105, 32, 116, 104, 101, 114, 101]).\n",
            "[72, 105, 32, 84, 104, 101, 114, 101]",
        ),
        (
            "## Main\nShow bytesLjust([65, 66], 5, 46).\n",
            "[65, 66, 46, 46, 46]",
        ),
        (
            "## Main\nShow bytesCenter([65, 66], 5, 42).\n",
            "[42, 42, 65, 66, 42]",
        ),
        (
            "## Main\nShow bytesZfill([45, 52, 50], 6).\n",
            "[45, 48, 48, 48, 52, 50]",
        ),
        ("## Main\nShow bytesIsAlnum([65, 49, 122]).\n", "true"),
    ];
    let dir = std::env::temp_dir().join(format!("logos_c_bascii_{}", std::process::id()));
    for (i, (src, want)) in cases.iter().enumerate() {
        let c =
            compile_to_c(src).unwrap_or_else(|e| panic!("compile_to_c failed for {src:?}: {e:?}"));
        let _ = std::fs::create_dir_all(&dir);
        let cpath = dir.join(format!("c{i}.c"));
        let bin = dir.join(format!("b{i}"));
        std::fs::write(&cpath, &c).unwrap();
        let out = std::process::Command::new("gcc")
            .args(["-O2", "-o"])
            .arg(&bin)
            .arg(&cpath)
            .arg("-lm")
            .output()
            .expect("run gcc");
        assert!(
            out.status.success(),
            "bytes-ascii gcc failed for {src:?}:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let run = std::process::Command::new(&bin).output().expect("run");
        assert_eq!(
            String::from_utf8_lossy(&run.stdout).trim(),
            *want,
            "{src:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// (9) The TEMPORAL battery: the whole Moment/Date/Time surface must compile with gcc AND match the
/// interpreter through the C backend — parse_rfc3339, the calendar extractors, ISO week, quarter,
/// RFC-3339 formatting, date_of/time_of + their Show forms, Moment Show, and Moment arithmetic. This
/// is what makes the temporal `time.*` capabilities honestly `aot_c`. Skipped when gcc is absent.
#[test]
fn c_backend_temporal_compiles_and_runs() {
    let gcc = std::process::Command::new("gcc").arg("--version").output();
    if gcc.map(|o| !o.status.success()).unwrap_or(true) {
        eprintln!("skip: gcc unavailable");
        return;
    }
    let ts = "parse_timestamp(\"2024-03-15T10:30:45Z\")";
    let cases: Vec<(String, &str)> = vec![
        (format!("## Main\nShow year_of({ts}).\n"), "2024"),
        (format!("## Main\nShow month_of({ts}).\n"), "3"),
        (format!("## Main\nShow day_of({ts}).\n"), "15"),
        (format!("## Main\nShow hour_of({ts}).\n"), "10"),
        (format!("## Main\nShow minute_of({ts}).\n"), "30"),
        (format!("## Main\nShow second_of({ts}).\n"), "45"),
        (format!("## Main\nShow weekday_of({ts}).\n"), "5"),
        (format!("## Main\nShow week_of({ts}).\n"), "11"),
        (format!("## Main\nShow quarter_of({ts}).\n"), "1"),
        (
            format!("## Main\nShow format_timestamp({ts}).\n"),
            "2024-03-15T10:30:45Z",
        ),
        (format!("## Main\nShow date_of({ts}).\n"), "2024-03-15"),
        (format!("## Main\nShow time_of({ts}).\n"), "10:30:45"),
        (format!("## Main\nShow {ts}.\n"), "2024-03-15 10:30"),
        (
            format!("## Main\nLet t be {ts}.\nShow seconds_between(t, add_seconds(t, 3600)).\n"),
            "3600",
        ),
        // Pre-epoch instant: floored calendar math must still yield a valid date + clock.
        (
            "## Main\nShow year_of(parse_timestamp(\"1969-12-31T23:59:59Z\")).\n".to_string(),
            "1969",
        ),
    ];
    let dir = std::env::temp_dir().join(format!("logos_c_temporal_{}", std::process::id()));
    for (i, (src, want)) in cases.iter().enumerate() {
        let c =
            compile_to_c(src).unwrap_or_else(|e| panic!("compile_to_c failed for {src:?}: {e:?}"));
        let _ = std::fs::create_dir_all(&dir);
        let cpath = dir.join(format!("t{i}.c"));
        let bin = dir.join(format!("tb{i}"));
        std::fs::write(&cpath, &c).unwrap();
        let out = std::process::Command::new("gcc")
            .args(["-O2", "-o"])
            .arg(&bin)
            .arg(&cpath)
            .arg("-lm")
            .output()
            .expect("run gcc");
        assert!(
            out.status.success(),
            "temporal gcc failed for {src:?}:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let run = std::process::Command::new(&bin).output().expect("run");
        assert_eq!(
            String::from_utf8_lossy(&run.stdout).trim(),
            *want,
            "{src:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
