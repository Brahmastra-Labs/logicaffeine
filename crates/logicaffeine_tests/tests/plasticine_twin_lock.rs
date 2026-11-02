//! plasticine_twin_lock — a LOGOS program and the Spatial app it is compared with on the Plasticine chain are
//! twins: the same operation, the same shape, the same tiling and parallelization.
//!
//! A twin is how the bench compares LOGOS with Spatial on Plasticine: our compiler recognizes an ordinary LOGOS
//! program as a tensor operation and prints it in the structure of Spatial's own regression app (DotProduct, TPCHQ6,
//! OuterProduct, GDA, BlackScholes, Kmeans), with the parameters of the Spatial variant it stands beside — so the two
//! runs on Tungsten differ only in who wrote them. The tests hold:
//!
//! * the LOGOS programs compute what they are recognized as, on the VM — Black-Scholes and k-means against plain-Rust
//!   references of Spatial's own approximations and of its accelerator's tie rule;
//! * a recognized term is typed as the VM types it (`Int` arithmetic exact, `/` truncating, a `Real` beside an `Int`
//!   promoting it), calls the program's pure functions inlined (an `If`'s continuations a select, recursion and
//!   mutation refused) and `abs`, and evaluates as a DAG exactly as it does as a tree, only the arm a select takes;
//! * TPC-H Q6 written in LOGOS is a zip sum — a guarded sum over four aligned columns — whose term is the query of
//!   Spatial's `TPCHQ6` app; Gaussian discriminant analysis is a scatter accumulation whose row term is the query of
//!   `GDA` once a select's common operand is hoisted out of its arms; Black-Scholes is a zip map — each option priced
//!   from its six columns — whose term, its five functions inlined, is the query of `BlackScholes`; k-means is Lloyd's
//!   algorithm with the distance of `Kmeans`;
//! * every recognizer reads the natural spellings, refuses a near miss by name, and refuses an array of another
//!   element type than the app computes in;
//! * each twin is printed in Spatial's own app structure over the program's own data, refusing parameters that do
//!   not divide its shape, a value Spatial's 32-bit `Int` would wrap or its 32-bit `Float` could not hold, and a
//!   division its host would trap on;
//! * a dot-product row joins the model as the `1 x 1 x N` GEMM (its `op` the GEMM's `loop_kk`), and a row of a kind
//!   the model does not describe (outer product, TPC-H Q6, GDA, Black-Scholes, k-means) is listed uncalibrated with
//!   the reason;
//! * in the toolchain ledger (`benchmarks/silicon/plasticine_toolchain.json`, written by
//!   `scripts/plasticine/twins.py record`), every run of Spatial's DotProduct / TPCHQ6 / OuterProduct / GDA /
//!   BlackScholes / Kmeans variants has exactly one LOGOS twin at the same knobs (runtime arguments included) on the
//!   same mesh and toolchain, compiled with the app's own PIR arguments, and back; each twin row's own LOGOS source is
//!   recognized at the row's shape and lowers, at the row's knobs, to the row's app. Whose cycles are lower is the
//!   data the report prints beside each pair — never asserted here, so a loss is a row, not a RED.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::plasticine_report::{
    blackscholes_query, calibration, calibration_row, gda_query, kmeans_distance_query, logos_blackscholes_program,
    logos_dot_program, logos_gda_program, logos_kmeans_program, logos_outer_program, logos_tpchq6_program,
    lower_blackscholes_source, lower_dot_source, lower_gda_source, lower_kmeans_source, lower_outer_source,
    lower_tpchq6_source, target_for_mesh, tpchq6_query, UNMODELED_KINDS,
};
use logicaffeine_compile::codegen_sva::spatial_emit::{
    emit_spatial_zip_map_test, emit_spatial_zip_sum_test, GdaPar, KmeansPar, MapColumn, MapPar, OuterPar, ZipColumn,
    ZipPar,
};
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::tensor_recognize::{
    hoist_common_operand, recognize_dot, recognize_kmeans, recognize_outer, recognize_scatter, recognize_zip_map,
    recognize_zip_sum, Axis, DotReject, Fill, KmeansReject, OuterReject, ScatterReject, Scalar, Stop, TermDag, Ties, Ty,
    ZipMapReject, ZipOp, ZipReject, ZipTerm,
};
use logicaffeine_compile::ui_bridge::with_parsed_program;
use serde_json::{json, Value};

fn dot_of(src: &str) -> Result<(usize, bool), DotReject> {
    with_parsed_program(src, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("parses");
        recognize_dot(stmts, interner).map(|s| (s.n, s.a != s.b))
    })
}

fn outer_of(src: &str) -> Result<(usize, usize), OuterReject> {
    with_parsed_program(src, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("parses");
        recognize_outer(stmts, interner).map(|s| (s.m, s.n))
    })
}

#[test]
fn the_logos_twins_compute_what_they_are_recognized_as() {
    let dot = |n: i64| (0..n).map(|t| (t % 7) * (t % 5)).sum::<i64>();
    for n in [1, 64, 4096] {
        let run = vm_outcome(&logos_dot_program(n as usize));
        assert_eq!((run.output.trim(), run.error.as_deref()), (dot(n).to_string().as_str(), None), "dot over {n}");
    }
    let outer = |rows: i64, cols: i64| {
        (0..rows)
            .flat_map(|i| (0..cols).map(move |j| (i * cols + j + 1) * (i % 7) * (j % 5)))
            .sum::<i64>()
    };
    for (rows, cols) in [(1, 1), (16, 8), (32, 64)] {
        let run = vm_outcome(&logos_outer_program(rows as usize, cols as usize));
        assert_eq!(
            (run.output.trim(), run.error.as_deref()),
            (outer(rows, cols).to_string().as_str(), None),
            "outer {rows} x {cols}"
        );
    }
}

#[test]
fn the_logos_dot_product_is_recognized_with_its_length() {
    assert_eq!(dot_of(&logos_dot_program(4096)), Ok((4096, true)));
    assert_eq!(dot_of(&logos_dot_program(65536)), Ok((65536, true)));
}

#[test]
fn a_loop_that_is_not_a_dot_product_is_refused_by_name() {
    let sum = logos_dot_program(64).replace("item (i + 1) of a * item (i + 1) of b", "item (i + 1) of a");
    assert!(sum.contains("Set s to s + item (i + 1) of a."), "the mutation applies");
    assert_eq!(dot_of(&sum), Err(DotReject::NoProduct));
    let shifted = logos_dot_program(64).replace("item (i + 1) of b", "item (i + 2) of b");
    assert_eq!(dot_of(&shifted), Err(DotReject::NotAligned), "a[i] · b[i+1] is not a dot product");
    let no_loop = "## Main\nLet s be 3.\nShow s.\n";
    assert_eq!(dot_of(no_loop), Err(DotReject::NoLoop));
}

/// A dot-product loop over `a` and `b` (eight items each, `s` the accumulator), its preamble and body given.
fn dot_loop(preamble: &str, cond: &str, body: &str) -> String {
    let body: String = body.lines().map(|l| format!("    {l}\n")).collect();
    format!(
        "## Main\nLet mutable a be a new Seq of Int.\nLet mutable b be a new Seq of Int.\nLet mutable p be 0.\n\
         While p is less than 8:\n    Push p to a.\n    Push p to b.\n    Set p to p + 1.\n{preamble}While {cond}:\n{body}Show s.\n"
    )
}

#[test]
fn every_spelling_of_a_dot_product_is_read_and_every_near_miss_refused_by_name() {
    let start = "Let mutable s be 0.\nLet mutable i be 0.\n";
    let step = "Set i to i + 1.";
    let cases: &[(&str, String, Result<(usize, bool), DotReject>)] = &[
        ("the canonical loop", dot_loop(start, "i is less than 8", &format!("Set s to s + item (i + 1) of a * item (i + 1) of b.\n{step}")), Ok((8, true))),
        ("the accumulator on the right", dot_loop(start, "i is less than 8", &format!("Set s to item (i + 1) of a * item (i + 1) of b + s.\n{step}")), Ok((8, true))),
        ("the product named first", dot_loop(start, "i is less than 8", &format!("Let t be item (i + 1) of a * item (i + 1) of b.\nSet s to s + t.\n{step}")), Ok((8, true))),
        ("the index named first", dot_loop(start, "i is less than 8", &format!("Let k be i + 1.\nSet s to s + item k of a * item k of b.\n{step}")), Ok((8, true))),
        ("the step first", dot_loop(start, "i is less than 8", &format!("{step}\nSet s to s + item i of a * item i of b.")), Ok((8, true))),
        ("a one-based counter to at most", dot_loop("Let mutable s be 0.\nLet mutable i be 1.\n", "i is at most 8", &format!("Set s to s + item i of a * item i of b.\n{step}")), Ok((8, true))),
        ("a square", dot_loop(start, "i is less than 8", &format!("Set s to s + item (i + 1) of a * item (i + 1) of a.\n{step}")), Ok((8, false))),
        ("half the vectors", dot_loop("Let mutable s be 0.\nLet mutable i be 4.\n", "i is less than 8", &format!("Set s to s + item (i + 1) of a * item (i + 1) of b.\n{step}")), Ok((4, true))),
        ("a counter set long before", dot_loop("Let mutable s be 0.\nLet mutable i be 0.\nShow s.\n", "i is less than 8", &format!("Set s to s + item (i + 1) of a * item (i + 1) of b.\n{step}")), Err(DotReject::NotCounted)),
        ("a counter stepped by two", dot_loop(start, "i is less than 8", "Set s to s + item (i + 1) of a * item (i + 1) of b.\nSet i to i + 2."), Err(DotReject::NotCounted)),
        ("a counter stepped twice", dot_loop(start, "i is less than 8", &format!("Set s to s + item (i + 1) of a * item (i + 1) of b.\n{step}\n{step}")), Err(DotReject::NotCounted)),
        ("a sum", dot_loop(start, "i is less than 8", &format!("Set s to s + item (i + 1) of a.\n{step}")), Err(DotReject::NoProduct)),
        ("a product of an item and the counter", dot_loop(start, "i is less than 8", &format!("Set s to s + item (i + 1) of a * i.\n{step}")), Err(DotReject::NoProduct)),
        ("a strided read", dot_loop(start, "i is less than 4", &format!("Set s to s + item (2 * i + 1) of a * item (2 * i + 1) of b.\n{step}")), Err(DotReject::NotAligned)),
        ("a reversed read", dot_loop(start, "i is less than 8", &format!("Set s to s + item (i + 1) of a * item (8 - i) of b.\n{step}")), Err(DotReject::NotAligned)),
        ("a quadratic index", dot_loop(start, "i is less than 2", &format!("Set s to s + item (i * i + 1) of a * item (i * i + 1) of b.\n{step}")), Err(DotReject::NonAffineIndex)),
        ("a print in the loop", dot_loop(start, "i is less than 8", &format!("Set s to s + item (i + 1) of a * item (i + 1) of b.\nShow s.\n{step}")), Err(DotReject::ExtraWork)),
        ("a second accumulation", dot_loop(start, "i is less than 8", &format!("Set s to s + item (i + 1) of a * item (i + 1) of b.\nSet s to s + 1.\n{step}")), Err(DotReject::ExtraWork)),
        ("an accumulator from five", dot_loop("Let mutable s be 5.\nLet mutable i be 0.\n", "i is less than 8", &format!("Set s to s + item (i + 1) of a * item (i + 1) of b.\n{step}")), Err(DotReject::NotFromZero)),
    ];
    for (what, src, want) in cases {
        assert_eq!(&dot_of(src), want, "{what}:\n{src}");
    }
    assert_eq!(dot_of(&logos_outer_program(4, 4)), Err(DotReject::NoProduct), "an outer product's checksum loop");
    let real = dot_loop(start, "i is less than 8", &format!("Set s to s + item (i + 1) of a * item (i + 1) of b.\n{step}"))
        .replace("a new Seq of Int.\nLet mutable b be a new Seq of Int.", "a new Seq of Real.\nLet mutable b be a new Seq of Real.")
        .replace("Push p to a.\n    Push p to b.", "Push p / 1.0 to a.\n    Push p / 1.0 to b.");
    assert!(real.contains("Seq of Real") && real.contains("p / 1.0"));
    assert_eq!(dot_of(&real), Err(DotReject::NotInt), "a dot product of Real columns is not the app's Int dot product");
}

/// An outer-product nest over `a` (four items), `b` (three) and `c` (twelve), its inner body given.
fn outer_nest(i_body_before: &str, j_body: &str) -> String {
    let j_body: String = j_body.lines().map(|l| format!("        {l}\n")).collect();
    let before: String = i_body_before.lines().map(|l| format!("    {l}\n")).collect();
    format!(
        "## Main\nLet mutable a be a new Seq of Int.\nLet mutable b be a new Seq of Int.\nLet mutable c be a new Seq of Int.\n\
         Let mutable p be 0.\nWhile p is less than 12:\n    Push p to a.\n    Push p to b.\n    Push 0 to c.\n    Set p to p + 1.\n\
         Let mutable i be 0.\nWhile i is less than 4:\n{before}    Let mutable j be 0.\n    While j is less than 3:\n{j_body}    Set i to i + 1.\nShow item 1 of c.\n"
    )
}

#[test]
fn every_spelling_of_an_outer_product_is_read_and_every_near_miss_refused_by_name() {
    let step = "Set j to j + 1.";
    let cases: &[(&str, String, Result<(usize, usize), OuterReject>)] = &[
        ("the canonical nest", outer_nest("", &format!("Let idx be i * 3 + j + 1.\nSet item idx of c to item (i + 1) of a * item (j + 1) of b.\n{step}")), Ok((4, 3))),
        ("the index inline", outer_nest("", &format!("Set item (i * 3 + j + 1) of c to item (i + 1) of a * item (j + 1) of b.\n{step}")), Ok((4, 3))),
        ("the product named first", outer_nest("", &format!("Let v be item (i + 1) of a * item (j + 1) of b.\nSet item (i * 3 + j + 1) of c to v.\n{step}")), Ok((4, 3))),
        ("a row offset named in the outer loop", outer_nest("Let r be i * 3.", &format!("Set item (r + j + 1) of c to item (i + 1) of a * item (j + 1) of b.\n{step}")), Ok((4, 3))),
        ("a row operand named in the outer loop", outer_nest("Let x be item (i + 1) of a.", &format!("Set item (i * 3 + j + 1) of c to x * item (j + 1) of b.\n{step}")), Ok((4, 3))),
        ("stored transposed", outer_nest("", &format!("Set item (j * 4 + i + 1) of c to item (i + 1) of a * item (j + 1) of b.\n{step}")), Err(OuterReject::NotOuterLayout)),
        ("the operands swapped", outer_nest("", &format!("Set item (i * 3 + j + 1) of c to item (j + 1) of a * item (i + 1) of b.\n{step}")), Err(OuterReject::NotOuterLayout)),
        ("a row stride wider than a row", outer_nest("", &format!("Set item (i * 4 + j + 1) of c to item (i + 1) of a * item (j + 1) of b.\n{step}")), Err(OuterReject::NotOuterLayout)),
        ("a rank-1 update", outer_nest("", &format!("Let idx be i * 3 + j + 1.\nSet item idx of c to item idx of c + item (i + 1) of a * item (j + 1) of b.\n{step}")), Err(OuterReject::Accumulates)),
        ("a sum", outer_nest("", &format!("Set item (i * 3 + j + 1) of c to item (i + 1) of a + item (j + 1) of b.\n{step}")), Err(OuterReject::NoProduct)),
        ("no store", outer_nest("", &format!("Show item (i + 1) of a * item (j + 1) of b.\n{step}")), Err(OuterReject::NoStore)),
        ("an inner counter stepped by two", outer_nest("", "Set item (i * 3 + j + 1) of c to item (i + 1) of a * item (j + 1) of b.\nSet j to j + 2."), Err(OuterReject::NotCounted)),
        ("a print in the inner loop", outer_nest("", &format!("Set item (i * 3 + j + 1) of c to item (i + 1) of a * item (j + 1) of b.\nShow j.\n{step}")), Err(OuterReject::ExtraWork)),
        ("a print in the outer loop", outer_nest("Show i.", &format!("Set item (i * 3 + j + 1) of c to item (i + 1) of a * item (j + 1) of b.\n{step}")), Err(OuterReject::ExtraWork)),
    ];
    for (what, src, want) in cases {
        assert_eq!(&outer_of(src), want, "{what}:\n{src}");
    }
    let canonical = outer_nest("", &format!("Set item (i * 3 + j + 1) of c to item (i + 1) of a * item (j + 1) of b.\n{step}"));
    let real = canonical.replace("Let mutable c be a new Seq of Int.", "Let mutable c be a new Seq of Real.");
    assert!(real != canonical);
    assert_eq!(outer_of(&real), Err(OuterReject::NotInt), "a product stored into a Real array is not the app's Int product");
    assert_eq!(outer_of(&logos_dot_program(8)), Err(OuterReject::NotDoubleNested), "a dot product");
    assert_eq!(dot_of("## Main\nShow 1.\n"), Err(DotReject::NoLoop));
    assert_eq!(outer_of("## Main\nShow 1.\n"), Err(OuterReject::NoLoopNest));
}

#[test]
fn the_logos_outer_product_is_recognized_with_its_shape() {
    assert_eq!(outer_of(&logos_outer_program(256, 128)), Ok((256, 128)));
    assert_eq!(outer_of(&logos_outer_program(64, 64)), Ok((64, 64)));
}

#[test]
fn an_outer_product_stored_transposed_is_refused_by_name() {
    let src = logos_outer_program(32, 16);
    let transposed = src.replace("Let idx be i * cols + j + 1.", "Let idx be j * rows + i + 1.");
    assert!(transposed.contains("j * rows + i + 1"), "the mutation applies");
    assert_eq!(outer_of(&transposed), Err(OuterReject::NotOuterLayout));
    let summed = src.replace("item (i + 1) of a * item (j + 1) of b", "item (i + 1) of a + item (j + 1) of b");
    assert_eq!(outer_of(&summed), Err(OuterReject::NoProduct));
}

#[test]
fn the_dot_twin_is_spatials_dot_product_structure() {
    let par = ZipPar { ts: 16384, ip: 16, op: 2 };
    let (n, class, spatial) = lower_dot_source(&logos_dot_program(65536), par).unwrap();
    assert!(spatial.contains("val a = Array.tabulate(N) { i => (i % 7).to[T] }"), "the program's own data:\n{spatial}");
    let (load_a, load_b) = (spatial.find("aBlk load a(").unwrap(), spatial.find("bBlk load b(").unwrap());
    assert!(load_a < load_b, "the columns load in the dot product's own order, as Spatial's app loads them");
    assert_eq!((n, class.as_str()), (65536, "Logos_dot_65536_ts16384_ip16_op2"));
    for line in [
        "@spatial class Logos_dot_65536_ts16384_ip16_op2 extends SpatialTest",
        "Reduce(accO)(N by ts par op) { i =>",
        "aBlk load a(i::i+ts par ip)",
        "bBlk load b(i::i+ts par ip)",
        "Parallel {",
        "val part = Reg[T]",
        "Reduce(part)(ts par ip) { ii => aBlk(ii) * bBlk(ii) } { _ + _ }",
        "val ts = 16384",
        "val op = 2",
    ] {
        assert!(spatial.contains(line), "{line}\n{spatial}");
    }
    assert!(lower_dot_source(&logos_dot_program(1000), ZipPar { ts: 256, ip: 16, op: 1 }).is_err(), "ts must divide N");
    assert!(lower_dot_source(&logos_dot_program(4096), ZipPar { ts: 1024, ip: 5, op: 1 }).is_ok(), "Spatial's own DotProduct_6 runs ip = 5");
}

#[test]
fn the_outer_twin_is_spatials_outer_product_structure() {
    let par = OuterPar { ts1: 64, ts2: 32, ip: 16, ip1: 16, ip2: 1, op1: 1, op2: 2 };
    let (m, n, class, spatial) = lower_outer_source(&logos_outer_program(256, 256), par).unwrap();
    assert_eq!((m, n, class.as_str()), (256, 256, "Logos_outer_256x256_ts64x32_ip16x16x1_op1x2"));
    for line in [
        "Foreach(M by ts1 par op1) { i =>",
        "b1 load vec1(i::i+ts1 par ip)",
        "Foreach(N by ts2 par op2) { j =>",
        "b2 load vec2(j::j+ts2 par ip)",
        "Foreach(ts1 par ip2, ts2 par ip1) { (ii, jj) => outTile(ii, jj) = b1(ii) * b2(jj) }",
        "out(i::i+ts1, j::j+ts2 par ip) store outTile",
    ] {
        assert!(spatial.contains(line), "{line}\n{spatial}");
    }
    assert!(lower_outer_source(&logos_outer_program(100, 64), par).is_err(), "ts1 must divide M");
}

fn zip_of(src: &str) -> Result<(usize, Vec<String>, ZipTerm), ZipReject> {
    with_parsed_program(src, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("parses");
        recognize_zip_sum(stmts, interner).map(|z| {
            (z.n, z.operands.iter().map(|o| interner.resolve(*o).to_string()).collect(), z.term)
        })
    })
}

#[test]
fn the_logos_tpchq6_query_is_a_zip_sum_of_its_four_columns_with_spatials_query() {
    let (n, columns, term) = zip_of(&logos_tpchq6_program(1024)).expect("TPC-H Q6 is a zip sum");
    assert_eq!(n, 1024);
    assert_eq!(columns, ["dates", "discts", "quants", "prices"], "the columns in reading order");
    assert_eq!(term, tpchq6_query(), "the query of Spatial's TPCHQ6 app");
}

#[test]
fn the_logos_tpchq6_query_computes_its_terms_sum_on_the_vm() {
    for n in [1, 100, 1024] {
        let run = vm_outcome(&logos_tpchq6_program(n));
        let want: i64 = (0..n as i64).map(|t| tpchq6_query().eval(&[t % 256; 4]).expect("in range")).sum();
        assert_eq!((run.output.trim(), run.error.as_deref()), (want.to_string().as_str(), None), "TPC-H Q6 over {n}");
    }
}

/// A zip-sum loop over `a` and `b` (eight items each, `s` the accumulator), its body given.
fn zip_loop(body: &str) -> String {
    dot_loop("Let mutable s be 0.\nLet mutable i be 0.\n", "i is less than 8", body)
}

#[test]
fn every_spelling_of_a_zip_sum_is_read_and_every_near_miss_refused_by_name() {
    let step = "Set i to i + 1.";
    let ok = |body: &str, cols: &[&str]| {
        let got = zip_of(&zip_loop(&format!("{body}\n{step}"))).unwrap_or_else(|e| panic!("{body}: {e:?}"));
        assert_eq!((got.0, got.1.iter().map(String::as_str).collect::<Vec<_>>()), (8, cols.to_vec()), "{body}");
        got.2
    };
    assert_eq!(ok("Set s to s + item (i + 1) of a.", &["a"]), ZipTerm::Read(0));
    ok("If item (i + 1) of a is greater than 3:\n    Set s to s + item (i + 1) of b.", &["a", "b"]);
    ok("If item (i + 1) of a is at most 3:\n    Set s to s + 1.\nOtherwise:\n    Set s to s + item (i + 1) of b * 2.", &["a", "b"]);
    ok("Let x be item (i + 1) of a - item (i + 1) of b.\nSet s to s + x * x.", &["a", "b"]);
    let halves = ok("Set s to s + item (i + 1) of a / 2.", &["a"]);
    assert_eq!(halves, ZipTerm::Op(ZipOp::Div, Box::new(ZipTerm::Read(0)), Box::new(ZipTerm::Const(2))), "an Int quotient");
    ok("Set s to s + abs(item (i + 1) of a - 4) ** 2.", &["a"]);
    let refused = |body: &str| zip_of(&zip_loop(&format!("{body}\n{step}")));
    assert_eq!(refused("Set s to s + item (i + 1) of a * i."), Err(ZipReject::UnsupportedTerm), "the counter is not a column");
    assert_eq!(refused("Set s to s + item (i + 1) of a % 2."), Err(ZipReject::UnsupportedTerm), "no remainder in a term");
    assert_eq!(refused("Set s to s + item (i + 1) of a * 0.5."), Err(ZipReject::NotInt), "a Real term is not the app's Int sum");
    assert_eq!(refused("Set s to s + (item (i + 1) of a is greater than 3)."), Err(ZipReject::UnsupportedTerm), "a comparison is not a number");
    assert_eq!(refused("If s is less than 9:\n    Set s to s + item (i + 1) of a."), Err(ZipReject::UnsupportedTerm), "the guard reads the sum");
    assert_eq!(refused("Set s to s + item (i + 1) of a * item (i + 2) of b."), Err(ZipReject::NotAligned));
    assert_eq!(refused("If item (i + 1) of a is greater than 3:\n    Set s to s + 1.\n    Show s."), Err(ZipReject::NoAccumulation), "an arm doing more than one accumulation");
    assert_eq!(refused("Set s to s + item (i + 1) of a.\nPush 1 to b."), Err(ZipReject::ExtraWork));
}

#[test]
fn the_tpchq6_twin_is_spatials_tpchq6_structure_over_the_programs_own_columns() {
    let par = ZipPar { ts: 64, ip: 16, op: 2 };
    let (n, class, spatial) = lower_tpchq6_source(&logos_tpchq6_program(1024), par).expect("the twin");
    assert_eq!((n, class.as_str()), (1024, "Logos_tpchq6_1024_ts64_ip16_op2"));
    for line in [
        "Reduce(accO)(N by ts par op) { i =>",
        "Parallel {",
        "datesBlk load dates(i::i+ts par ip)",
        "disctsBlk load discts(i::i+ts par ip)",
        "quantsBlk load quants(i::i+ts par ip)",
        "pricesBlk load prices(i::i+ts par ip)",
        "Reduce(part)(ts par ip) { ii => mux(",
        "val dates = Array.tabulate(N) { i => (i % 256).to[T] }",
        "val gold = Array.tabulate(N) { i => if (",
    ] {
        assert!(spatial.contains(line), "{line}\n{spatial}");
    }
    assert!(lower_tpchq6_source(&logos_dot_program(1024), par).is_err(), "a dot product is not TPC-H Q6");
}

#[test]
fn a_twin_whose_32_bit_int_would_wrap_is_refused_and_table_4s_tpchq6_is_not() {
    let par = ZipPar { ts: 16, ip: 16, op: 1 };
    let columns = |base: u32| {
        logos_dot_program(64)
            .replace("p % 7 to a", &format!("p % 7 + {base} to a"))
            .replace("p % 5 to b", &format!("p % 5 + {base} to b"))
    };
    let element = lower_dot_source(&columns(100_000), par).unwrap_err();
    assert!(element.contains("element") && element.contains("32-bit"), "a product past 32 bits: {element}");
    let sum = lower_dot_source(&columns(40_000), par).unwrap_err();
    assert!(sum.contains("the sum") && sum.contains("32-bit"), "products that fit, a sum that does not: {sum}");
    assert!(lower_dot_source(&columns(40), par).is_ok());
    // Table 4's 6.4 million records: the query keeps only quantities below 24, so the revenue stays near 10^8.
    assert!(lower_tpchq6_source(&logos_tpchq6_program(6_400_000), ZipPar { ts: 64, ip: 16, op: 1 }).is_ok());
}

/// A toolchain row as the ledger records one, with the fields the calibration reads.
fn chain_row(app: &str, kind: &str, shape: Value, params: Value) -> Value {
    json!({
        "key": format!("{app}/20x20"), "app": app, "kind": kind, "shape": shape, "params": params,
        "mesh": { "rows": 20, "cols": 20 },
        "hybrid_network": { "cycles": 6056, "pass": true }, "ideal_network": { "cycles": 6030, "pass": true },
        "pir": { "PCU": { "used": 7 }, "PMU": { "used": 5 }, "DAG": { "used": 3 }, "MC": { "used": 3 } },
        "plastiroute": { "best_score": 0.51 },
    })
}

#[test]
fn a_dot_row_is_modeled_as_the_one_by_one_by_n_gemm_its_op_the_gemms_loop_kk() {
    let t = target_for_mesh(20, 20).expect("the toolchain's mesh");
    let shape = json!({ "m": 1, "n": 1, "k": 65536 });
    let modeled = |row: &Value| calibration_row(&t, row).expect("modeled")["modeled"]["cycles"].as_u64().unwrap();
    for ip in [16, 1] {
        let mut by_op = Vec::new();
        for op in [1, 2, 3] {
            let dot = chain_row("DotProduct_x", "dot", shape.clone(), json!({ "N": 65536, "ts": 16384, "ip": ip, "op": op }));
            let gemm = chain_row(
                "GEMM_x",
                "gemm",
                shape.clone(),
                json!({ "ts": 16384, "its": 1, "ip": ip, "loop_kk": op, "loop_i": 1, "loop_j": 1, "loop_ii": 1, "loop_jj": 1 }),
            );
            assert_eq!(modeled(&dot), modeled(&gemm), "ip={ip} op={op}");
            assert_eq!(calibration_row(&t, &dot).unwrap()["kind"], "dot");
            by_op.push(modeled(&dot));
        }
        // Sixteen lanes keep up with the memory, so more tiles at once cannot help; one lane is compute-bound,
        // so two tiles at once must model fewer cycles than one.
        if ip == 1 {
            assert!(by_op[1] < by_op[0], "a compute-bound dot with op=2 models fewer cycles than op=1: {by_op:?}");
        }
    }
}

/// Spatial streams a K-chunked reduction's operands chunk by chunk at the reduce's own lane width: the first chunk
/// is loaded before any compute starts, a chunk whose load outlasts its compute stalls the MAC for the
/// difference, and a reduce five lanes wide runs five, not the descriptor's sixteen.
#[test]
fn a_chunked_reduction_waits_for_its_first_chunk_and_runs_at_its_own_lane_width() {
    let t = target_for_mesh(20, 20).expect("the toolchain's mesh");
    let shape = json!({ "m": 1, "n": 1, "k": 65536 });
    for (ip, op) in [(1usize, 1usize), (1, 2), (5, 1), (5, 3), (16, 1), (16, 8)] {
        let row = chain_row("DotProduct_x", "dot", shape.clone(), json!({ "N": 65536, "ts": 16384, "ip": ip, "op": op }));
        let cal = calibration_row(&t, &row).expect("modeled");
        let m = &cal["modeled"];
        assert_eq!(m["lanes"], ip, "ip={ip}: the reduce runs at the app's own lane width");
        assert_eq!(cal["lanes_agree"], true, "ip={ip}");
        let chunk = (16384 + ip - 1) / ip;
        assert_eq!(m["operand_stall_cycles"], chunk, "ip={ip} op={op}: the first chunk loads before any compute");
        let in_sequence = (4 + op - 1) / op;
        let cycles = m["cycles"].as_u64().unwrap() as usize;
        assert!(cycles >= chunk * (in_sequence + 1), "ip={ip} op={op}: {cycles} cycles hide a chunk load");
    }
}

#[test]
fn an_outer_row_is_listed_uncalibrated_with_its_reason_never_joined_or_dropped() {
    let outer = chain_row(
        "OuterProduct_x",
        "outer",
        json!({ "m": 256, "n": 256, "k": 1 }),
        json!({ "M": 256, "N": 256, "ts1": 64, "ts2": 32, "ip": 16, "ip1": 16, "ip2": 1, "op1": 1, "op2": 1 }),
    );
    let dot = chain_row(
        "DotProduct_x",
        "dot",
        json!({ "m": 1, "n": 1, "k": 65536 }),
        json!({ "N": 65536, "ts": 16384, "ip": 16, "op": 1 }),
    );
    let t = target_for_mesh(20, 20).unwrap();
    let why = UNMODELED_KINDS.iter().find(|(k, _)| *k == "outer").expect("outer is unmodeled").1;
    assert!(calibration_row(&t, &outer).unwrap_err().contains(why), "an outer row is never joined with the GEMM model");
    let cal = calibration(&json!({ "rows": [outer, dot] })).expect("calibration");
    assert_eq!(cal["rows"].as_array().unwrap().len(), 1);
    assert_eq!(cal["rows"][0]["app"], "DotProduct_x");
    assert_eq!(cal["uncalibrated"], json!([{ "key": "OuterProduct_x/20x20", "app": "OuterProduct_x", "kind": "outer", "why": why }]));
}

/// A row whose routed (hybrid) network did not finish clean — it deadlocked, threw or failed its gold check, as the
/// recorder's `why` says — is listed uncalibrated with what happened, never joined with the model, which is calibrated
/// against routed cycles; a clean row is joined.
#[test]
fn a_routed_run_that_did_not_finish_clean_is_listed_uncalibrated_with_what_happened() {
    let dot = |app: &str, hybrid: Value| {
        let mut r = chain_row(app, "dot", json!({ "m": 1, "n": 1, "k": 65536 }), json!({ "N": 65536, "ts": 16384, "ip": 16, "op": 1 }));
        r["hybrid_network"] = hybrid;
        r
    };
    let rows = json!({ "rows": [
        dot("DotProduct_a", json!({ "cycles": 8853, "pass": false, "why": "deadlocked at cycle 8853" })),
        dot("DotProduct_b", json!({ "cycles": null, "pass": true, "why": "threw on cycle 166: /DUT/Top/net/router40: no route for packet 343" })),
        dot("DotProduct_c", json!({ "cycles": 6056, "pass": true })),
    ]});
    let cal = calibration(&rows).expect("calibration");
    let joined: Vec<&str> = cal["rows"].as_array().unwrap().iter().map(|r| r["app"].as_str().unwrap()).collect();
    assert_eq!(joined, ["DotProduct_c"]);
    let why: Vec<(&str, &str)> =
        cal["uncalibrated"].as_array().unwrap().iter().map(|u| (u["app"].as_str().unwrap(), u["why"].as_str().unwrap())).collect();
    assert_eq!(
        why,
        [
            ("DotProduct_a", "its routed-network simulation deadlocked at cycle 8853"),
            ("DotProduct_b", "its routed-network simulation threw on cycle 166: /DUT/Top/net/router40: no route for packet 343"),
        ]
    );
    let t = target_for_mesh(20, 20).unwrap();
    assert!(calibration_row(&t, &rows["rows"][0]).unwrap_err().contains("deadlocked at cycle 8853"), "never joined row by row either");
}

fn toolchain_ledger() -> Value {
    let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/plasticine_toolchain.json");
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))).expect("the ledger parses")
}

fn usize_of(v: &Value) -> usize {
    v.as_u64().expect("a size") as usize
}

#[test]
fn every_spatial_tensor_app_run_has_one_logos_twin_at_its_knobs_and_back() {
    let doc = toolchain_ledger();
    let rows: Vec<&Value> = doc["rows"].as_array().unwrap().iter().collect();
    let is = |r: &Value, prefix: &str| r["app"].as_str().unwrap().starts_with(prefix);
    let mut printed = Vec::new();
    for (kind, theirs_prefix, ours_prefix) in [
        ("dot", "DotProduct_", "Logos_dot_"),
        ("tpchq6", "TPCHQ6_", "Logos_tpchq6_"),
        ("outer", "OuterProduct_", "Logos_outer_"),
        ("gda", "GDA_", "Logos_gda_"),
        ("blackscholes", "BlackScholes_", "Logos_bs_"),
        ("kmeans", "Kmeans_", "Logos_kmeans_"),
    ] {
        let theirs: Vec<&Value> = rows.iter().copied().filter(|r| is(r, theirs_prefix)).collect();
        let ours: Vec<&Value> = rows.iter().copied().filter(|r| is(r, ours_prefix)).collect();
        assert!(
            theirs.iter().any(|r| r["app"] == format!("{theirs_prefix}0")),
            "Spatial's {theirs_prefix}0 is in the ledger — `scripts/plasticine/twins.py run` and `record` put it there"
        );
        for r in theirs.iter().chain(&ours) {
            assert_eq!(r["kind"], kind, "{}: recorded as a {kind} row", r["key"]);
        }
        let same = |a: &Value, b: &Value| a["params"] == b["params"] && a["mesh"] == b["mesh"] && a["toolchain"] == b["toolchain"];
        for t in &theirs {
            let twins: Vec<&&Value> = ours.iter().filter(|o| same(t, o)).collect();
            assert_eq!(twins.len(), 1, "{}: exactly one LOGOS twin at its knobs, mesh and toolchain", t["key"]);
            let command = |r: &Value| {
                let p = r["excerpts"]["gentst.sh"].as_str().unwrap_or_else(|| panic!("{}: its PIR command is kept", r["key"]));
                std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").join(p))
                    .unwrap_or_else(|e| panic!("{p}: {e}"))
            };
            assert_eq!(command(t), command(twins[0]), "{}: its twin ran with the app's own PIR arguments", t["key"]);
            assert!(t["app_source"].as_str().unwrap().contains("unmodified"), "{}: Spatial's own app", t["key"]);
            assert!(t.get("logos_source").is_none(), "{}: Spatial's app has no LOGOS source", t["key"]);
            // Both on the routed network when both finished clean there; otherwise both on the ideal network, which
            // every row finished clean, and what each routed run came to beside them.
            let clean = |r: &Value| r["hybrid_network"]["why"].is_null();
            let network = if clean(t) && clean(twins[0]) { "hybrid_network" } else { "ideal_network" };
            let (tc, oc) = (
                t[network]["cycles"].as_u64().unwrap_or_else(|| panic!("{}: {network} cycles", t["key"])),
                twins[0][network]["cycles"].as_u64().unwrap_or_else(|| panic!("{}: {network} cycles", twins[0]["key"])),
            );
            let routed = |r: &Value| r["hybrid_network"]["why"].as_str().map_or(String::new(), |w| format!(" (routed: {w})"));
            printed.push(format!(
                "{:<18} {:>9} cycles   {:<46} {:>9} cycles   x{:.3}  {network}{}{}",
                t["app"].as_str().unwrap(),
                tc,
                twins[0]["app"].as_str().unwrap(),
                oc,
                oc as f64 / tc as f64,
                routed(t),
                routed(twins[0])
            ));
        }
        for o in &ours {
            assert_eq!(theirs.iter().filter(|t| same(t, o)).count(), 1, "{}: the twin of exactly one Spatial run", o["key"]);
            let src = o["logos_source"].as_str().unwrap_or_else(|| panic!("{}: a twin row carries its LOGOS source", o["key"]));
            let p = &o["params"];
            let (app, recognized, declared) = if kind == "dot" || kind == "tpchq6" {
                let par = ZipPar { ts: usize_of(&p["ts"]), ip: usize_of(&p["ip"]), op: usize_of(&p["op"]) };
                let lower = if kind == "dot" { lower_dot_source } else { lower_tpchq6_source };
                let (n, class, _) = lower(src, par).unwrap_or_else(|e| panic!("{}: {e}", o["key"]));
                (class, vec![n], vec![usize_of(&p["N"])])
            } else if kind == "kmeans" {
                let par = KmeansPar {
                    ts: usize_of(&p["ts"]), ip: usize_of(&p["ip"]), op: usize_of(&p["op"]), mp1: usize_of(&p["mp1"]),
                    mp2: usize_of(&p["mp2"]), mp3: usize_of(&p["mp3"]),
                };
                let (n, k, d, iters, class, _) = lower_kmeans_source(src, par).unwrap_or_else(|e| panic!("{}: {e}", o["key"]));
                (class, vec![n, k, d, iters], vec![usize_of(&p["N"]), usize_of(&p["K"]), usize_of(&p["D"]), usize_of(&p["I"])])
            } else if kind == "blackscholes" {
                let par = MapPar { ts: usize_of(&p["ts"]), op: usize_of(&p["op"]), ip: usize_of(&p["ip"]) };
                let (n, class, _) = lower_blackscholes_source(src, par, None).unwrap_or_else(|e| panic!("{}: {e}", o["key"]));
                (class, vec![n], vec![usize_of(&p["N"])])
            } else if kind == "gda" {
                let par = GdaPar {
                    ts: usize_of(&p["ts"]), op: usize_of(&p["op"]), mp1: usize_of(&p["mp1"]), mp2: usize_of(&p["mp2"]),
                    ip: usize_of(&p["ip"]),
                };
                let (rows, cols, class, _) = lower_gda_source(src, par).unwrap_or_else(|e| panic!("{}: {e}", o["key"]));
                (class, vec![rows, cols], vec![usize_of(&p["R"]), usize_of(&p["C"])])
            } else {
                let par = OuterPar {
                    ts1: usize_of(&p["ts1"]), ts2: usize_of(&p["ts2"]), ip: usize_of(&p["ip"]), ip1: usize_of(&p["ip1"]),
                    ip2: usize_of(&p["ip2"]), op1: usize_of(&p["op1"]), op2: usize_of(&p["op2"]),
                };
                let (m, n, class, _) = lower_outer_source(src, par).unwrap_or_else(|e| panic!("{}: {e}", o["key"]));
                (class, vec![m, n], vec![usize_of(&p["M"]), usize_of(&p["N"])])
            };
            assert_eq!(recognized, declared, "{}: its LOGOS source is recognized at the row's shape", o["key"]);
            assert_eq!(o["app"], app.as_str(), "{}: the row's app is what its source lowers to at its knobs", o["key"]);
            let emitter = match kind {
                "outer" => "emit_spatial_outer_test",
                "gda" => "emit_spatial_gda_test",
                "blackscholes" => "emit_spatial_zip_map_test",
                "kmeans" => "emit_spatial_kmeans_test",
                _ => "emit_spatial_zip_sum_test",
            };
            assert!(o["app_source"].as_str().unwrap().contains(emitter), "{}: {emitter}", o["key"]);
        }
    }
    println!("{}", printed.join("\n"));
}

fn scatter_of(src: &str) -> Result<(usize, usize, Vec<(String, Axis)>, ZipTerm), ScatterReject> {
    with_parsed_program(src, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("parses");
        recognize_scatter(stmts, interner).map(|z| {
            let cols = z.operands.iter().map(|(o, a)| (interner.resolve(*o).to_string(), *a)).collect();
            (z.rows, z.cols, cols, z.term)
        })
    })
}

#[test]
fn a_selects_common_operand_is_hoisted_out_of_its_arms_exactly() {
    let op = |o, l, r| ZipTerm::Op(o, Box::new(l), Box::new(r));
    let sel = |c, a, b| ZipTerm::Select(Box::new(c), Box::new(a), Box::new(b));
    let cond = op(ZipOp::Eq, ZipTerm::Read(0), ZipTerm::Const(1));
    let arms = sel(cond.clone(), op(ZipOp::Sub, ZipTerm::Read(1), ZipTerm::Read(2)), op(ZipOp::Sub, ZipTerm::Read(1), ZipTerm::Read(3)));
    let hoisted = op(ZipOp::Sub, ZipTerm::Read(1), sel(cond.clone(), ZipTerm::Read(2), ZipTerm::Read(3)));
    assert_eq!(hoist_common_operand(arms.clone()), hoisted);
    let right = sel(cond.clone(), op(ZipOp::Mul, ZipTerm::Read(2), ZipTerm::Read(1)), op(ZipOp::Mul, ZipTerm::Read(3), ZipTerm::Read(1)));
    assert_eq!(hoist_common_operand(right), op(ZipOp::Mul, sel(cond.clone(), ZipTerm::Read(2), ZipTerm::Read(3)), ZipTerm::Read(1)));
    let different = sel(cond, op(ZipOp::Sub, ZipTerm::Read(1), ZipTerm::Read(2)), op(ZipOp::Add, ZipTerm::Read(1), ZipTerm::Read(3)));
    assert_eq!(hoist_common_operand(different.clone()), different, "arms of different operators keep their select");
    for row in [[0i64, 7, 3, 5], [1, 7, 3, 5], [1, -4, 9, 2], [0, 0, 0, 0]] {
        assert_eq!(arms.eval(&row), hoisted.eval(&row), "{row:?}");
    }
}

#[test]
fn the_logos_gda_is_a_scatter_accumulation_with_spatials_query() {
    let (rows, cols, columns, term) = scatter_of(&logos_gda_program(16, 8)).expect("GDA is a scatter accumulation");
    assert_eq!((rows, cols), (16, 8));
    let named: Vec<(&str, Axis)> = columns.iter().map(|(n, a)| (n.as_str(), *a)).collect();
    assert_eq!(named, [("y", Axis::Row), ("x", Axis::RowCol), ("mu1", Axis::Col), ("mu0", Axis::Col)]);
    assert_eq!(term, gda_query().1, "the query of Spatial's GDA app: x - (y == 1 ? mu1 : mu0)");
    assert_eq!(named.iter().map(|(_, a)| *a).collect::<Vec<_>>(), gda_query().0, "and its reading axes");
}

#[test]
fn the_logos_gda_computes_the_scatter_matrix_on_the_vm() {
    for (rows, cols) in [(1usize, 1usize), (4, 3), (16, 8)] {
        let x = |r: usize, c: usize| ((r * cols + c) % 256) as i64;
        let mu = |c: usize| (c % 2) as i64;
        let d = |r: usize, c: usize| x(r, c) - mu(c);
        let want: i64 = (0..cols * cols)
            .map(|t| (t as i64 + 1) * (0..rows).map(|r| d(r, t / cols) * d(r, t % cols)).sum::<i64>())
            .sum();
        let run = vm_outcome(&logos_gda_program(rows, cols));
        assert_eq!((run.output.trim(), run.error.as_deref()), (want.to_string().as_str(), None), "GDA {rows} x {cols}");
    }
}

#[test]
fn every_near_miss_of_a_scatter_accumulation_is_refused_by_name() {
    let src = logos_gda_program(8, 4);
    let transposed = src.replace("item (r * cols + c + 1) of x", "item (c * rows + r + 1) of x");
    assert!(transposed != src);
    assert_eq!(scatter_of(&transposed).unwrap_err(), ScatterReject::NotScatterLayout, "x read column-major");
    let two_vectors = src.replace("item (i + 1) of sub * item (j + 1) of sub", "item (i + 1) of sub * item (j + 1) of mu0");
    assert!(two_vectors != src);
    assert_eq!(scatter_of(&two_vectors).unwrap_err(), ScatterReject::NoRank1Update, "a rank-1 update of two vectors");
    let stored = src.replace("item idx of sigma + item (i + 1) of sub * item (j + 1) of sub", "item (i + 1) of sub * item (j + 1) of sub");
    assert!(stored != src);
    assert_eq!(scatter_of(&stored).unwrap_err(), ScatterReject::NoRank1Update, "the outer product stored, not accumulated");
    let from_one = src.replace("    Push 0 to sigma.", "    Push 1 to sigma.");
    assert!(from_one != src);
    assert_eq!(scatter_of(&from_one).unwrap_err(), ScatterReject::NotFromZero);
    let extra = src.replace("    Set r to r + 1.", "    Show r.\n    Set r to r + 1.");
    assert!(extra != src);
    assert_eq!(scatter_of(&extra).unwrap_err(), ScatterReject::ExtraWork);
    assert_eq!(scatter_of(&logos_outer_program(4, 4)).unwrap_err(), ScatterReject::NoRowStore, "an outer product's inner loop writes C[i·n + j], not a row at its counter");
    assert_eq!(scatter_of(&logos_dot_program(8)).unwrap_err(), ScatterReject::NoRowLoop, "a dot product has no loop inside its loop");
}

#[test]
fn the_gda_twin_is_spatials_gda_structure_over_the_programs_own_data() {
    let par = GdaPar { ts: 128, op: 1, mp1: 2, mp2: 2, ip: 16 };
    let (rows, cols, class, spatial) = lower_gda_source(&logos_gda_program(256, 128), par).expect("the twin");
    assert_eq!((rows, cols, class.as_str()), (256, 128, "Logos_gda_256x128_ts128_op1_mp2x2_ip16"));
    for line in [
        "@spatial class Logos_gda_256x128_ts128_op1_mp2x2_ip16 extends SpatialTest",
        "mu1Tile load mu1(0 :: C par ip)",
        "mu0Tile load mu0(0 :: C par ip)",
        "MemReduce(sigmaOut par ip)(R by ts par op) { r =>",
        "yTile load y(r :: r + ts par ip)",
        "xTile load x(r :: r + ts, 0 :: C par ip)",
        "MemReduce(sigmaBlk par ip)(ts par mp1) { rr =>",
        "Foreach(C par ip) { cc => subTile(cc) = xTile(rr, cc) - mux(yTile(rr) === 1.to[T], mu1Tile(cc), mu0Tile(cc)) }",
        "Foreach(C by 1 par mp2) { ii =>",
        "Foreach(C par ip) { jj => sigmaTile(ii, jj) = subTile(ii) * subTile(jj) }",
        "sigma(0 :: C, 0 :: C par ip) store sigmaOut",
        "val x = Array.tabulate(R * C) { i => (i % 256).to[T] }",
    ] {
        assert!(spatial.contains(line), "{line}\n{spatial}");
    }
    assert!(lower_gda_source(&logos_gda_program(100, 8), GdaPar { ts: 32, op: 1, mp1: 1, mp2: 1, ip: 8 }).is_err(), "ts must divide R");
    assert!(lower_gda_source(&logos_dot_program(64), par).is_err(), "a dot product is not GDA");
    let wide = logos_gda_program(8, 4).replace("Push p % 256 to x.", "Push p % 256 + 100000 to x.");
    let small = GdaPar { ts: 8, op: 1, mp1: 1, mp2: 1, ip: 4 };
    let wraps = lower_gda_source(&wide, small).unwrap_err();
    assert!(wraps.contains("covariance") && wraps.contains("32-bit"), "a covariance past 32 bits is refused: {wraps}");
    assert!(lower_gda_source(&logos_gda_program(8, 4), small).is_ok());
}

/// Black-Scholes as Spatial's BlackScholes app prices an option (`BlkSchlsEqEuroNoDiv`, its `CNDF` and the Taylor
/// approximations of `spatial.lang.api.MathAPI`), in plain Rust and in doubles — the reference the LOGOS program, its
/// recognized term and the transcribed query are each held to.
fn black_scholes(price: f64, strike: f64, rate: f64, vol: f64, time: f64, otype: i64) -> f64 {
    let exp_taylor = |x: f64| {
        if x < -3.5 {
            0.0
        } else if x < -1.2 {
            x * 0.1 + 0.35
        } else {
            1.0 + x + x * x / 2.0 + x * x * x / 6.0 + x * x * x * x / 24.0 + x * x * x * x * x / 120.0
        }
    };
    let log_taylor = |x: f64| {
        let xx = x - 1.0;
        xx - xx * xx / 2.0 + xx * xx * xx / 3.0 - xx * xx * xx * xx / 4.0
    };
    let sqrt_approx = |x: f64| match x {
        x if x < 2.0 => 1.0 + (x - 1.0) / 2.0 - (x - 1.0) * (x - 1.0) / 8.0 + (x - 1.0) * (x - 1.0) * (x - 1.0) / 16.0,
        x if x < 10.0 => x * 0.22 + 1.0,
        x if x < 100.0 => x * 0.08 + 2.5,
        x if x < 1000.0 => x * 0.028 + 8.0,
        x if x < 10000.0 => x * 0.008 + 20.0,
        x => x * 0.0002 + 300.0,
    };
    let cndf = |x: f64| {
        let ax = x.abs();
        let n_prime = exp_taylor(ax.powf(2.0) * -0.05) * 0.39894228040143270286;
        let k = 1.0 / (ax * 0.2316419 + 1.0);
        let (k2, k3, k4, k5) = (k.powf(2.0), k.powf(2.0) * k, k.powf(2.0) * k * k, k.powf(2.0) * k * k * k);
        let l1 = k2 * -0.356563782 + k3 * 1.781477937 + k4 * -1.821255978 + k5 * 1.330274429 + k * 0.31938153;
        let l0 = l1 * n_prime;
        if x < 0.0 {
            l0
        } else {
            0.0 - l0 + 1.0
        }
    };
    let d_num = (rate + vol.powf(2.0) * 0.5) * time + log_taylor(price / strike);
    let den = vol * sqrt_approx(time);
    let d = d_num / den.powf(2.0);
    let (nd1, nd2) = (cndf(d), cndf(d - den));
    let future = strike * exp_taylor((0.0 - rate) * time);
    if otype == 0 {
        future * (0.0 - nd2 + 1.0) - price * (0.0 - nd1 + 1.0)
    } else {
        price * nd1 - future * nd2
    }
}

/// The program's option `t`: (price, strike, rate, volatility, time, type), as its fill loop pushes them.
fn option_row(t: i64) -> (f64, f64, f64, f64, f64, i64) {
    let r = |m: i64, s: i64, d: f64| (t % m + s) as f64 / d;
    (r(97, 50, 100.0), r(89, 50, 100.0), r(13, 1, 100.0), r(41, 10, 100.0), r(23, 2, 4.0), t % 2)
}

fn near(a: f64, b: f64, rel: f64) -> bool {
    (a - b).abs() <= rel * a.abs().max(b.abs()).max(1.0)
}

#[test]
fn the_logos_blackscholes_prices_every_option_as_spatials_app_does() {
    for n in [1i64, 7, 256] {
        let run = vm_outcome(&logos_blackscholes_program(n as usize));
        assert_eq!(run.error.as_deref(), None, "Black-Scholes over {n}");
        let got: f64 = run.output.trim().parse().unwrap_or_else(|e| panic!("{:?}: {e}", run.output));
        let want: f64 = (0..n)
            .map(|t| {
                let (p, s, r, v, tm, ty) = option_row(t);
                (t + 1) as f64 * black_scholes(p, s, r, v, tm, ty)
            })
            .sum();
        assert!(near(got, want, 1e-9), "Black-Scholes over {n}: the VM shows {got}, the reference {want}");
    }
}

/// The program's recognized zip map: `(n, columns in reading order with their types, term, output type, output)`.
fn zip_map_of(src: &str) -> Result<(usize, Vec<(String, Ty)>, ZipTerm, Ty, String), ZipMapReject> {
    with_parsed_program(src, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("parses");
        recognize_zip_map(stmts, interner).map(|m| {
            let cols = m.operands.iter().zip(&m.types).map(|(o, t)| (interner.resolve(*o).to_string(), *t)).collect();
            (m.n, cols, m.term, m.ty, interner.resolve(m.out).to_string())
        })
    })
}

#[test]
fn the_logos_blackscholes_is_a_zip_map_whose_term_is_spatials_query() {
    let (n, columns, term, ty, out) = zip_map_of(&logos_blackscholes_program(512)).expect("Black-Scholes is a zip map");
    assert_eq!((n, ty, out.as_str()), (512, Ty::Real, "out"));
    let named: Vec<(&str, Ty)> = columns.iter().map(|(c, t)| (c.as_str(), *t)).collect();
    assert_eq!(
        named,
        [("prices", Ty::Real), ("strikes", Ty::Real), ("rates", Ty::Real), ("vols", Ty::Real), ("times", Ty::Real), ("types", Ty::Int)],
        "the columns in reading order: the call's arguments"
    );
    assert_eq!(term, blackscholes_query(), "the five functions inlined are the query of Spatial's BlackScholes app");
    let dag = TermDag::of(&term);
    assert!(dag.nodes.len() < 400, "two CNDFs and their Taylor polynomials, each subterm once: {} nodes", dag.nodes.len());
    for t in 0..200 {
        let (p, s, r, v, tm, typ) = option_row(t);
        let row = [Scalar::Real(p), Scalar::Real(s), Scalar::Real(r), Scalar::Real(v), Scalar::Real(tm), Scalar::Int(typ)];
        let want = black_scholes(p, s, r, v, tm, typ);
        let ints = (i64::MIN as i128, i64::MAX as i128);
        for got in [term.value(&row, ints, f64::MAX), dag.value(&row, ints, f64::MAX)] {
            match got {
                Ok(Scalar::Real(x)) => assert!(near(x, want, 1e-12), "option {t}: {x} against {want}"),
                other => panic!("option {t}: {other:?}"),
            }
        }
    }
}

/// A map loop over `a` (Real), `ints` (Int) and `out` (Real), eight items each, with the functions `fns` and the
/// loop body `body`.
fn map_loop(fns: &str, body: &str) -> String {
    let body: String = body.lines().map(|l| format!("    {l}\n")).collect();
    format!(
        "{fns}## Main\nLet mutable a be a new Seq of Real.\nLet mutable ints be a new Seq of Int.\nLet mutable out be a new Seq of Real.\n\
         Let mutable p be 0.\nWhile p is less than 8:\n    Push (p % 5 + 1) / 4.0 to a.\n    Push p % 3 to ints.\n    Push 0.0 to out.\n    Set p to p + 1.\n\
         Let mutable i be 0.\nWhile i is less than 8:\n{body}Show item 1 of out.\n"
    )
}

#[test]
fn every_spelling_of_a_zip_map_is_read_and_every_near_miss_refused_by_name() {
    let step = "Set i to i + 1.";
    let read = |src: String| zip_map_of(&src);
    let ok = |fns: &str, body: &str| read(map_loop(fns, &format!("{body}\n{step}"))).unwrap_or_else(|e| panic!("{body}: {e:?}"));
    let op = |o, l, r| ZipTerm::Op(o, Box::new(l), Box::new(r));
    let (n, cols, term, ty, _) = ok("", "Set item (i + 1) of out to item (i + 1) of a * 2.0.");
    assert_eq!((n, cols, term, ty), (8, vec![("a".to_string(), Ty::Real)], op(ZipOp::Mul, ZipTerm::Read(0), ZipTerm::real(2.0)), Ty::Real));
    let (_, cols, term, _, _) = ok("", "Let x be item (i + 1) of ints.\nSet item (i + 1) of out to item (i + 1) of a + x.");
    assert_eq!(cols.iter().map(|(_, t)| *t).collect::<Vec<_>>(), [Ty::Real, Ty::Int], "an Int column beside a Real one");
    assert_eq!(term, op(ZipOp::Add, ZipTerm::Read(0), ZipTerm::Read(1)));
    let clip = "## To clip (x: Real) -> Real:\n    If x is greater than 1.0:\n        Return 1.0.\n    Otherwise:\n        Let y be x * 2.0.\n        Return y.\n\n";
    let (_, _, term, _, _) = ok(clip, "Set item (i + 1) of out to clip(item (i + 1) of a).");
    let sel = ZipTerm::Select(
        Box::new(op(ZipOp::Gt, ZipTerm::Read(0), ZipTerm::real(1.0))),
        Box::new(ZipTerm::real(1.0)),
        Box::new(op(ZipOp::Mul, ZipTerm::Read(0), ZipTerm::real(2.0))),
    );
    assert_eq!(term, sel, "an If with an Otherwise is the select of its arms");
    let early = "## To clip (x: Real) -> Real:\n    If x is greater than 1.0:\n        Return 1.0.\n    Let y be x * 2.0.\n    Return y.\n\n";
    assert_eq!(ok(early, "Set item (i + 1) of out to clip(item (i + 1) of a).").2, sel, "an early Return is the same select");
    let twice = "## To sq (x: Real) -> Real:\n    Return x * x.\n\n## To quad (x: Real) -> Real:\n    Return sq(sq(x)).\n\n";
    let (_, _, term, _, _) = ok(twice, "Set item (i + 1) of out to quad(item (i + 1) of a).");
    let sq = |x: ZipTerm| op(ZipOp::Mul, x.clone(), x);
    assert_eq!(term, sq(sq(ZipTerm::Read(0))), "a function calling a function, each inlined");
    let refused = |fns: &str, body: &str| read(map_loop(fns, &format!("{body}\n{step}")));
    assert_eq!(refused("", "Set item (i + 2) of out to item (i + 1) of a."), Err(ZipMapReject::NotAligned), "stored one past what it reads");
    assert_eq!(refused("", "Set item (i + 1) of out to item (i + 1) of a * item (i + 2) of a."), Err(ZipMapReject::NotAligned));
    assert_eq!(refused("", "Set item (i + 1) of out to item (i + 1) of out * 2.0."), Err(ZipMapReject::UnsupportedTerm), "reads its own output");
    assert_eq!(refused("", "Set item (i + 1) of out to sqrt(item (i + 1) of a)."), Err(ZipMapReject::UnsupportedTerm), "a builtin a term does not have");
    assert_eq!(refused("", "Set item (i + 1) of out to item (i + 1) of a * i."), Err(ZipMapReject::UnsupportedTerm), "the counter is not a column");
    assert_eq!(refused("", "Set item (i + 1) of out to item (i + 1) of ints."), Err(ZipMapReject::UnsupportedTerm), "an Int stored into a Real column");
    assert_eq!(refused("", "Set item (i + 1) of out to 3.0."), Err(ZipMapReject::UnsupportedTerm), "a fill reads no column");
    assert_eq!(refused("", "Set item (i + 1) of out to item (i * i + 1) of a."), Err(ZipMapReject::NonAffineIndex));
    assert_eq!(refused("", "Set item (i + 1) of out to item (i + 1) of a.\nShow i."), Err(ZipMapReject::ExtraWork));
    assert_eq!(
        read(map_loop("", "Set item (i + 1) of out to item (i + 1) of a.\nSet i to i + 2.")),
        Err(ZipMapReject::NotCounted)
    );
    let recursive = "## To down (x: Real) -> Real:\n    If x is less than 1.0:\n        Return x.\n    Return down(x - 1.0).\n\n";
    assert_eq!(refused(recursive, "Set item (i + 1) of out to down(item (i + 1) of a)."), Err(ZipMapReject::UnsupportedTerm), "a function that calls itself");
    let mutating = "## To twice (x: Real) -> Real:\n    Let mutable y be x.\n    Set y to y * 2.0.\n    Return y.\n\n";
    assert_eq!(refused(mutating, "Set item (i + 1) of out to twice(item (i + 1) of a)."), Err(ZipMapReject::UnsupportedTerm), "a function that mutates");
    let partial = "## To pos (x: Real) -> Real:\n    If x is greater than 0.0:\n        Return x.\n\n";
    assert_eq!(refused(partial, "Set item (i + 1) of out to pos(item (i + 1) of a)."), Err(ZipMapReject::UnsupportedTerm), "a path with no Return");
    let wrong_result = "## To twice (x: Real) -> Int:\n    Return x * 2.0.\n\n";
    assert_eq!(refused(wrong_result, "Set item (i + 1) of out to twice(item (i + 1) of a)."), Err(ZipMapReject::UnsupportedTerm), "a Real returned as an Int");
    let int_param = "## To half (x: Int) -> Real:\n    Return x / 2.0.\n\n";
    assert_eq!(refused(int_param, "Set item (i + 1) of out to half(item (i + 1) of a)."), Err(ZipMapReject::UnsupportedTerm), "a Real argument for an Int parameter");
    assert_eq!(zip_map_of("## Main\nShow 1.\n"), Err(ZipMapReject::NoLoop));
    assert_eq!(zip_map_of(&logos_dot_program(8)), Err(ZipMapReject::NoStore), "a dot product stores nothing");
}

#[test]
fn a_dag_computes_what_its_term_does_once_per_subterm_and_only_the_arm_taken() {
    let op = |o, l, r| ZipTerm::Op(o, Box::new(l), Box::new(r));
    let x = ZipTerm::Read(0);
    let sq = op(ZipOp::Mul, x.clone(), x.clone());
    let guarded = ZipTerm::Select(
        Box::new(op(ZipOp::Eq, x.clone(), ZipTerm::Const(0))),
        Box::new(ZipTerm::Const(7)),
        Box::new(op(ZipOp::Div, op(ZipOp::Add, sq.clone(), sq.clone()), x.clone())),
    );
    let dag = TermDag::of(&guarded);
    assert_eq!(dag.nodes.len(), 8, "x, 0, x == 0, 7, x·x once, x·x + x·x, the quotient, the select");
    let sq_node = dag.nodes.iter().position(|n| *n == logicaffeine_compile::tensor_recognize::DagNode::Op(ZipOp::Mul, 0, 0)).unwrap();
    assert_eq!(dag.uses[sq_node], 2, "x·x is an operand twice");
    let ints = (i32::MIN as i128, i32::MAX as i128);
    for v in [-9i64, -1, 0, 1, 3, 46340, 46341] {
        let row = [Scalar::Int(v)];
        assert_eq!(dag.value(&row, ints, f64::MAX), guarded.value(&row, ints, f64::MAX), "x = {v}");
    }
    assert_eq!(guarded.value(&[Scalar::Int(0)], ints, f64::MAX), Ok(Scalar::Int(7)), "the arm with the zero divisor is not taken");
    assert_eq!(op(ZipOp::Div, ZipTerm::Const(1), x.clone()).value(&[Scalar::Int(0)], ints, f64::MAX), Err(Stop::ZeroDivisor));
    assert_eq!(guarded.value(&[Scalar::Int(46341)], ints, f64::MAX), Err(Stop::IntRange), "46341² leaves 32 bits");
    assert_eq!(op(ZipOp::Div, ZipTerm::Const(-7), ZipTerm::Const(2)).eval(&[]), Some(-3), "Int division truncates toward zero, as the VM's");
    assert_eq!(op(ZipOp::Pow, ZipTerm::Const(3), ZipTerm::Const(-1)).value(&[], ints, f64::MAX), Err(Stop::NegativeExponent));
    let big = op(ZipOp::Mul, ZipTerm::real(1e30), ZipTerm::real(1e30));
    assert_eq!(big.value(&[], ints, f32::MAX as f64), Err(Stop::RealRange), "past the 32-bit Float");
    assert_eq!(big.value(&[], ints, f64::MAX), Ok(Scalar::Real(1e30 * 1e30)));
}

#[test]
fn the_blackscholes_twin_is_spatials_blackscholes_structure_over_the_programs_own_columns() {
    let par = MapPar { ts: 256, op: 1, ip: 16 };
    let (n, class, spatial) =
        lower_blackscholes_source(&logos_blackscholes_program(1024), par, Some(" --split-algo=bfs")).expect("the twin");
    assert_eq!((n, class.as_str()), (1024, "Logos_bs_1024_ts256_op1_ip16"));
    for line in [
        "@spatial class Logos_bs_1024_ts256_op1_ip16 extends SpatialTest {",
        "  override def pirArgs = super.pirArgs + \" --split-algo=bfs\"",
        "def zipmap(typesIn: Array[Int], pricesIn: Array[Float], strikesIn: Array[Float], ratesIn: Array[Float], volsIn: Array[Float], timesIn: Array[Float]) = {",
        "val out = DRAM[Float](N)",
        "Foreach(N by ts par op) { i =>",
        "val typesBlk = FIFO[Int](ts)",
        "val pricesBlk = FIFO[Float](ts)",
        "val outBlk = FIFO[Float](ts)",
        "typesBlk load types(i::i+ts par ip)",
        "timesBlk load times(i::i+ts par ip)",
        "Foreach(ts par ip) { j =>",
        "val pricesV = pricesBlk.deq",
        "val typesV = typesBlk.deq",
        "outBlk.enq(mux(",
        "out(i::i+ts par ip) store outBlk",
        "val types = Array.tabulate(N) { i => (i % 2).to[Int] }",
        "val prices = Array.tabulate(N) { i => (((i % 97) + 50).to[Float] / 100.0.to[Float]) }",
        "val result = zipmap(types, prices, strikes, rates, vols, times)",
        "abs(r - g) <= 0.001.to[Float] * abs(g) + 0.001.to[Float]",
        "println(\"PASS: \" + cksum + \" (Logos_bs_1024_ts256_op1_ip16)\")",
    ] {
        assert!(spatial.contains(line), "{line}\n{spatial}");
    }
    let at = |s: &str| spatial.find(s).unwrap_or_else(|| panic!("{s}"));
    assert!(at("typesBlk load") < at("pricesBlk load"), "loaded in the order the program declares its columns, as the app loads its own");
    assert!(at("val pricesV") < at("val typesV"), "dequeued in the order the pricing call reads them");
    assert!(spatial.contains("abs(") && spatial.contains(" ** 2)") && spatial.contains("(0.0.to[Float] - "));
    assert!(!spatial.contains("** 2.0"), "a square is Spatial's integer power, as its BlackScholes writes it");
    assert!(spatial.contains("(typesV === 0.to[Int])"), "the type compared as an Int");
    assert!(spatial.len() < 40_000, "the term printed as its DAG, each shared subterm once: {} bytes", spatial.len());
    let vals = spatial
        .lines()
        .filter(|l| l.trim_start().strip_prefix("val t").map_or(false, |r| r.starts_with(|c: char| c.is_ascii_digit())))
        .count();
    assert!(vals > 10 && vals % 2 == 0, "every shared subterm bound once in the accelerator and once in the gold: {vals}");
    let (_, _, plain) = lower_blackscholes_source(&logos_blackscholes_program(1024), par, None).expect("the twin");
    assert!(!plain.contains("pirArgs"), "no override without the variant's");
    assert!(lower_blackscholes_source(&logos_blackscholes_program(1000), par, None).unwrap_err().contains("must divide"));
    assert!(lower_blackscholes_source(&logos_blackscholes_program(1024), MapPar { ts: 8, op: 1, ip: 16 }, None).is_err(), "a map wider than its tile");
    assert!(lower_blackscholes_source(&logos_dot_program(1024), par, None).unwrap_err().contains("not Black-Scholes"));
    let other = logos_blackscholes_program(1024).replace("* 0.2316419", "* 0.2316418");
    assert!(lower_blackscholes_source(&other, par, None).unwrap_err().contains("not Black-Scholes"), "one constant changed");
}

#[test]
fn a_zip_map_twin_refuses_what_spatials_32_bit_types_or_its_host_cannot_compute() {
    let op = |o, l, r| ZipTerm::Op(o, Box::new(l), Box::new(r));
    let par = MapPar { ts: 4, op: 1, ip: 4 };
    let real = MapColumn { name: "a".into(), ty: Ty::Real, fill: Fill::RealPattern { modulus: 5, shift: 1, divisor: 4f64.to_bits() } };
    let int = MapColumn { name: "b".into(), ty: Ty::Int, fill: Fill::Pattern { modulus: 3, shift: 0 } };
    let emit = |cols: &[MapColumn], term: &ZipTerm| emit_spatial_zip_map_test(8, cols, &(0..cols.len()).collect::<Vec<_>>(), term, "out", par, None, "X");
    let scaled = op(ZipOp::Mul, ZipTerm::Read(0), ZipTerm::real(2.0));
    assert!(emit(&[real.clone()], &scaled).is_ok());
    let huge = op(ZipOp::Mul, op(ZipOp::Mul, ZipTerm::Read(0), ZipTerm::real(1e30)), ZipTerm::real(1e30));
    assert!(emit(&[real.clone()], &huge).unwrap_err().contains("32-bit Float"), "past the Float's range");
    let wide = op(ZipOp::Mul, op(ZipOp::Add, ZipTerm::Read(0), ZipTerm::Const(100_000)), ZipTerm::Const(100_000));
    assert!(emit(&[int.clone()], &wide).unwrap_err().contains("32-bit Int"), "past the Int's range");
    let quotient = op(ZipOp::Div, ZipTerm::Const(12), ZipTerm::Read(0));
    assert!(emit(&[int.clone()], &quotient).unwrap_err().contains("divides by zero"), "b is 0 at element 0");
    let guarded = ZipTerm::Select(
        Box::new(op(ZipOp::Eq, ZipTerm::Read(0), ZipTerm::Const(0))),
        Box::new(ZipTerm::Const(1)),
        Box::new(quotient.clone()),
    );
    assert!(guarded.value(&[Scalar::Int(0)], (i32::MIN as i128, i32::MAX as i128), f64::MAX).is_ok(), "the VM never divides");
    assert!(emit(&[int.clone()], &guarded).unwrap_err().contains("host"), "but the app's host would evaluate the quotient");
    assert!(emit(&[int.clone(), int.clone()], &scaled).unwrap_err().contains("twice"), "two columns of one name");
    let mixed = ZipTerm::Select(
        Box::new(op(ZipOp::Lt, ZipTerm::Read(1), ZipTerm::real(-0.5))),
        Box::new(op(ZipOp::Add, ZipTerm::Read(0), ZipTerm::Read(1))),
        Box::new(op(ZipOp::Mul, ZipTerm::Read(1), ZipTerm::Const(-3))),
    );
    let text = emit(&[int.clone(), real.clone()], &mixed).expect("an Int column beside a Real one");
    for line in [
        "outBlk.enq(mux((aV < (-0.5).to[Float]), (bV.to[Float] + aV), (aV * (-3.0).to[Float])))",
        "(if ((a(i) < (-0.5).to[Float])) (b(i).to[Float] + a(i)) else (a(i) * (-3.0).to[Float]))",
        "val b = Array.tabulate(N) { i => (i % 3).to[Int] }",
        "val a = Array.tabulate(N) { i => (((i % 5) + 1).to[Float] / 4.0.to[Float]) }",
    ] {
        assert!(text.contains(line), "an Int converted where a Real is added, a negative literal a literal, the host's if:\n{line}\n{text}");
    }
    let named = |name: &str| MapColumn { name: name.into(), ..real.clone() };
    for bad in ["N", "val", "Blk", "x y", "t0"] {
        let col = named(bad);
        let term = if bad == "t0" { op(ZipOp::Mul, op(ZipOp::Add, ZipTerm::Read(0), ZipTerm::Read(0)), op(ZipOp::Add, ZipTerm::Read(0), ZipTerm::Read(0))) } else { scaled.clone() };
        assert!(emit(&[col], &term).is_err(), "`{bad}` is not a column name the app can declare");
    }
    let unknown = MapColumn { fill: Fill::Unknown, ..real.clone() };
    assert!(emit(&[unknown], &scaled).unwrap_err().contains("host data"), "data the compiler cannot see");
    assert!(emit_spatial_zip_map_test(8, &[real.clone()], &[1], &scaled, "out", par, None, "X").is_err(), "not an order of the columns");
}

#[test]
fn a_blackscholes_row_is_listed_uncalibrated_with_its_reason() {
    let why = UNMODELED_KINDS.iter().find(|(k, _)| *k == "blackscholes").expect("blackscholes is unmodeled").1;
    let row = chain_row("BlackScholes_x", "blackscholes", json!({ "m": 1, "n": 1, "k": 131072 }), json!({ "N": 131072, "ts": 256, "op": 1, "ip": 16 }));
    let t = target_for_mesh(20, 20).unwrap();
    assert!(calibration_row(&t, &row).unwrap_err().contains(why));
}

/// Lloyd's k-means as Spatial's Kmeans accelerator clusters (the last of equally near centroids), in plain Rust over the
/// program's data (`q % 97` row-major): the final centroids, `k × f`.
fn lloyd(n: usize, k: usize, f: usize, iters: usize, first: bool) -> Vec<i64> {
    let pts: Vec<i64> = (0..n * f).map(|q| (q % 97) as i64).collect();
    let mut cts = pts[..k * f].to_vec();
    for _ in 0..iters {
        let (mut sums, mut counts) = (vec![0i64; k * f], vec![0i64; k]);
        for p in 0..n {
            let dists: Vec<i64> = (0..k).map(|c| (0..f).map(|d| (pts[p * f + d] - cts[c * f + d]).pow(2)).sum()).collect();
            let min = *dists.iter().min().unwrap();
            let best = if first { (0..k).find(|c| dists[*c] == min) } else { (0..k).rev().find(|c| dists[*c] == min) }.unwrap();
            for d in 0..f {
                sums[best * f + d] += pts[p * f + d];
            }
            counts[best] += 1;
        }
        for c in 0..k {
            for d in 0..f {
                cts[c * f + d] = if counts[c] == 0 { 0 } else { sums[c * f + d] / counts[c] };
            }
        }
    }
    cts
}

fn checksum(v: &[i64]) -> i64 {
    v.iter().enumerate().map(|(t, x)| (t as i64 + 1) * x).sum()
}

#[test]
fn the_logos_kmeans_clusters_as_spatials_accelerator_does() {
    // 200 points of one feature, 100 centroids: the 98th point repeats the first, so two centroids start equal — every
    // point near them is a tie, which the last of the two wins, and the first is left empty.
    let pts: Vec<i64> = (0..200).map(|q| q % 97).collect();
    assert_eq!(pts[0], pts[97]);
    assert!(lloyd(200, 100, 1, 2, false) != lloyd(200, 100, 1, 2, true), "the ties decide something");
    assert!(lloyd(200, 100, 1, 1, false).contains(&0), "a centroid left empty");
    for (n, k, f, iters) in [(8usize, 2usize, 3usize, 2usize), (16, 4, 5, 3), (40, 8, 7, 2), (12, 12, 2, 1), (200, 100, 1, 2)] {
        let run = vm_outcome(&logos_kmeans_program(n, k, f, iters));
        let want = lloyd(n, k, f, iters, false);
        assert_eq!((run.output.trim(), run.error.as_deref()), (checksum(&want).to_string().as_str(), None), "k-means {n} {k} {f} {iters}");
        let spec = kmeans_of(&logos_kmeans_program(n, k, f, iters)).expect("k-means");
        let pts: Vec<i64> = (0..n * f).map(|q| (q % 97) as i64).collect();
        assert_eq!(spec.run(&pts, (i64::MIN as i128, i64::MAX as i128)), Ok(want), "the recognized clustering, run");
    }
}

fn kmeans_of(src: &str) -> Result<logicaffeine_compile::tensor_recognize::KmeansSpec, KmeansReject> {
    with_parsed_program(src, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("parses");
        recognize_kmeans(stmts, interner)
    })
}

#[test]
fn the_logos_kmeans_is_recognized_with_its_shape_and_spatials_distance() {
    let src = logos_kmeans_program(256, 64, 63, 2);
    let spec = kmeans_of(&src).expect("k-means");
    assert_eq!((spec.n, spec.k, spec.f, spec.iters, spec.ties), (256, 64, 63, 2, Ties::Last));
    assert_eq!(spec.distance, kmeans_distance_query(), "the squared difference Spatial's Kmeans app sums");
    let names = with_parsed_program(&src, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("parses");
        let s = recognize_kmeans(stmts, interner).unwrap();
        (interner.resolve(s.points).to_string(), interner.resolve(s.centroids).to_string())
    });
    assert_eq!(names, ("pts".to_string(), "cts".to_string()));
}

#[test]
fn every_near_miss_of_a_kmeans_is_refused_by_name() {
    let src = logos_kmeans_program(16, 4, 3, 2);
    let changed = |from: &str, to: &str| {
        let out = src.replace(from, to);
        assert!(out != src, "{from}");
        out
    };
    let first = kmeans_of(&changed("dist is at most bestDist", "dist is less than bestDist")).expect("the first of equally near");
    assert_eq!(first.ties, Ties::First);
    let ties = logos_kmeans_program(200, 100, 1, 2).replace("dist is at most bestDist", "dist is less than bestDist");
    let run = vm_outcome(&ties);
    assert_eq!(run.output.trim(), checksum(&lloyd(200, 100, 1, 2, true)).to_string(), "the first of equally near, on the VM");
    let pts: Vec<i64> = (0..200).map(|q| q % 97).collect();
    assert_eq!(kmeans_of(&ties).unwrap().run(&pts, (i64::MIN as i128, i64::MAX as i128)), Ok(lloyd(200, 100, 1, 2, true)));
    let flipped = kmeans_of(&changed("If c is 0 or dist is at most bestDist:", "If dist is at most bestDist or c is 0:")).expect("either order");
    assert_eq!(flipped.ties, Ties::Last);
    let absolute = kmeans_of(&changed(
        "(item (p * f + d + 1) of pts - item (c * f + d + 1) of cts) ** 2",
        "abs(item (p * f + d + 1) of pts - item (c * f + d + 1) of cts)",
    ))
    .expect("a Manhattan distance is k-means too");
    assert!(absolute.distance != kmeans_distance_query());
    let cases: &[(&str, String, KmeansReject)] = &[
        ("seeded from the second point", changed("Push item (q + 1) of pts to cts.", "Push item (q + 2) of pts to cts."), KmeansReject::NotFromFirstPoints),
        ("counts never zeroed", changed("    While q is less than k:\n        Set item (q + 1) of counts to 0.\n        Set q to q + 1.\n", ""), KmeansReject::NotZeroed),
        ("an empty centroid divided by", changed("            If cnt is 0:\n                Set item (j * f + q + 1) of cts to 0.\n            Otherwise:\n                Set item (j * f + q + 1) of cts to item (j * f + q + 1) of sums / cnt.\n", "            Set item (j * f + q + 1) of cts to item (j * f + q + 1) of sums / cnt.\n"), KmeansReject::NoUpdate),
        ("the nearest distance not kept", changed("                Set bestDist to dist.", "                Set bestDist to c."), KmeansReject::NoArgmin),
        ("an empty centroid kept", changed("                Set item (j * f + q + 1) of cts to 0.", "                Set item (j * f + q + 1) of cts to item (j * f + q + 1) of cts."), KmeansReject::NoUpdate),
        ("the centroid read one feature on", changed("item (c * f + d + 1) of cts", "item (c * f + d + 2) of cts"), KmeansReject::NotRowLayout),
        ("a point counted twice", changed("of counts + 1.", "of counts + 2."), KmeansReject::NoAccumulation),
        ("a print among the centroids", changed("            Set c to c + 1.", "            Show c.\n            Set c to c + 1."), KmeansReject::ExtraWork),
        ("a distance over the counter", changed(") ** 2.", ") ** 2 * d."), KmeansReject::UnsupportedTerm),
        ("Real points", changed("Let mutable pts be a new Seq of Int.", "Let mutable pts be a new Seq of Real."), KmeansReject::NotInt),
        ("a centroid loop stepped by two", changed("            Set c to c + 1.", "            Set c to c + 2."), KmeansReject::NotCounted),
    ];
    for (what, s, want) in cases {
        assert_eq!(kmeans_of(s).unwrap_err(), *want, "{what}");
    }
    assert_eq!(kmeans_of(&logos_dot_program(8)).unwrap_err(), KmeansReject::NoAssignment, "a dot product");
    assert_eq!(kmeans_of("## Main\nShow 1.\n").unwrap_err(), KmeansReject::NoLoop);
}

#[test]
fn the_kmeans_twin_is_spatials_kmeans_structure_over_the_programs_own_points() {
    let par = KmeansPar { ts: 128, ip: 16, op: 1, mp1: 1, mp2: 1, mp3: 1 };
    let (n, k, d, iters, class, spatial) = lower_kmeans_source(&logos_kmeans_program(256, 64, 63, 2), par).expect("the twin");
    assert_eq!((n, k, d, iters, class.as_str()), (256, 64, 64, 2, "Logos_kmeans_256x64_k64_i2_ts128_ip16_op1_mp1x1x1"));
    for line in [
        "@spatial class Logos_kmeans_256x64_k64_i2_ts128_ip16_op1_mp1x1x1 extends SpatialTest { self =>",
        "  val K = 64\n  val D = 64\n  val ts = 128\n  val ip = 16\n  val op = 1\n  val mp1 = 1\n  val mp2 = 1\n  val mp3 = 1\n",
        "  override def runtimeArgs = \"2 256\"",
        "      cts load points(0::K, 0::D par ip)",
        "      Sequential.Foreach(iters by 1){ epoch =>",
        "        if (ip == 1) {\n          newCents.onlyblockcyclic.blockcyclic_Bs(Seq(63))",
        "        MemReduce(newCents par ip)(numPoints by ts par op){i =>",
        "          MemReduce(SRAM[X](K,D) par ip)(ts par mp1){pt =>",
        "            Foreach(K by 1 par mp2) { ct =>",
        "                mux(d==DM1, 0, ((pts(pt,d) - cts(ct,d)) ** 2))",
        "              mux(dists(ct) == minDist.value, ct, -1)\n            } { (a,b) => max(a,b) }",
        "              localCent(ct, d) = mux(ct == minCent.value, pts(pt,d), 0.to[X])",
        "        Foreach(K by 1 par mp3){ ct =>\n          Foreach(D-1 par ip){ d =>",
        "            cts(ct, d) = mux(cond, 0.to[X], newCents(ct,d) / mux(cond,1.to[X], centCount))",
        "      centroids(0::K, D par ip) store cts",
        "    val pts = Array.tabulate(N){i => Array.tabulate(D){d => if (d == DM1) 1.to[X] else ((i * DM1 + d) % 97).to[Int] }}",
        "        if (i==DM1) 0.to[X] else ((p1(i) - p2(i)) ** 2)",
        "        val idx = dists.zip(ii){(a,b) => pack(a,b) }.reduce{(a,b) => if (a._1 < b._1 || (a._1 == b._1 && a._2 > b._2)) a else b}._2",
        "        for (d <- 0 until DM1) {",
        "    val cksum = gold.zip(result) { (g, r) => g === r }.reduce { _ & _ }",
    ] {
        assert!(spatial.contains(line), "{line}\n{spatial}");
    }
    let first = logos_kmeans_program(256, 64, 63, 2).replace("dist is at most bestDist", "dist is less than bestDist");
    let (_, _, _, _, _, spatial) = lower_kmeans_source(&first, par).expect("the twin");
    assert!(spatial.contains("mux(dists(ct) == minDist.value, ct, K)\n            } { (a,b) => min(a,b) }"), "{spatial}");
    assert!(spatial.contains("if (a._1 < b._1 || (a._1 == b._1 && a._2 < b._2)) a else b"), "the first of equally near, on the host too, whichever way it folds");
    assert!(lower_kmeans_source(&logos_kmeans_program(200, 64, 63, 2), par).unwrap_err().contains("must divide"));
    let manhattan = logos_kmeans_program(256, 64, 63, 2)
        .replace("(item (p * f + d + 1) of pts - item (c * f + d + 1) of cts) ** 2", "abs(item (p * f + d + 1) of pts - item (c * f + d + 1) of cts)");
    assert!(lower_kmeans_source(&manhattan, par).unwrap_err().contains("not by the distance"));
    let wide = logos_kmeans_program(256, 64, 63, 2).replace("Push q % 97 to pts.", "Push q % 97 + 9000000 to pts.");
    let wraps = lower_kmeans_source(&wide, par).unwrap_err();
    assert!(wraps.contains("32-bit"), "a feature sum past 32 bits: {wraps}");
    assert!(lower_kmeans_source(&logos_dot_program(64), par).unwrap_err().contains("not k-means"));
}

#[test]
fn a_kmeans_row_is_listed_uncalibrated_with_its_reason() {
    let why = UNMODELED_KINDS.iter().find(|(k, _)| *k == "kmeans").expect("kmeans is unmodeled").1;
    let row = chain_row(
        "Kmeans_x",
        "kmeans",
        json!({ "m": 64, "n": 64, "k": 256 }),
        json!({ "K": 64, "D": 64, "ts": 128, "ip": 16, "op": 1, "mp1": 1, "mp2": 1, "mp3": 1, "I": 2, "N": 256 }),
    );
    let t = target_for_mesh(20, 20).unwrap();
    assert!(calibration_row(&t, &row).unwrap_err().contains(why));
}

#[test]
fn a_real_pattern_fill_is_its_elements_and_an_overridden_one_is_unknown() {
    let fill_of = |src: &str| {
        with_parsed_program(src, |parsed, interner| {
            let (stmts, _, _) = parsed.expect("parses");
            logicaffeine_compile::tensor_recognize::recognize_fill(stmts, interner.lookup("xs").expect("xs"))
        })
    };
    let base = "## Main\nLet mutable xs be a new Seq of Real.\nLet mutable p be 0.\nWhile p is less than 8:\n    Push (p % 5 + 2) / 4.0 to xs.\n    Set p to p + 1.\n";
    let fill = fill_of(&format!("{base}Show item 1 of xs.\n"));
    assert_eq!(fill, Fill::RealPattern { modulus: 5, shift: 2, divisor: 4f64.to_bits() });
    let run = vm_outcome(&format!("{base}Show item 4 of xs.\nShow item 8 of xs.\n"));
    let shown: Vec<f64> = run.output.split_whitespace().map(|v| v.parse().unwrap()).collect();
    assert_eq!(shown, [fill.element(3), fill.element(7)].map(|e| match e { Some(Scalar::Real(v)) => v, other => panic!("{other:?}") }));
    assert_eq!(fill_of(&format!("{base}Set item 2 of xs to 9.5.\nShow item 1 of xs.\n")), Fill::Unknown, "an override a pattern cannot carry");
    assert_eq!(fill_of("## Main\nLet mutable xs be a new Seq of Real.\nLet mutable p be 0.\nWhile p is less than 8:\n    Push (p * 5 % 8) / 8.0 to xs.\n    Set p to p + 1.\nShow item 1 of xs.\n"), Fill::Unknown, "a residue of a multiple of the counter is not a pattern");
    use logicaffeine_compile::spatial::ir::TensorInit;
    use logicaffeine_compile::spatial::passes::from_tensor_op::init_of;
    assert_eq!(init_of(&fill, 8), TensorInit::External, "the device inits leave a real pattern to run time, as data they cannot see");
}

/// A power whose exponent is a whole-number constant is printed as Spatial's own BlackScholes writes one, `x ** 2` — a
/// Scala `Int` exponent, which Spatial stages as multiplication — never as a power at the operand's type
/// (`x ** 2.0.to[Float]`), a transcendental operator its accelerator would compute instead; a fractional exponent stays
/// a `Float` power. Both printers hold to it: the zip map's (Black-Scholes) and the zip sum's (TPC-H Q6, the dot).
#[test]
fn a_whole_power_is_printed_as_spatials_integer_power() {
    let cols = [MapColumn { name: "v".into(), ty: Ty::Real, fill: Fill::RealPattern { modulus: 7, shift: 1, divisor: 4.0f64.to_bits() } }];
    let pow = |e: ZipTerm| ZipTerm::Op(ZipOp::Pow, Box::new(ZipTerm::Read(0)), Box::new(e));
    let map = |t: ZipTerm| {
        emit_spatial_zip_map_test(256, &cols, &[0], &t, "out", MapPar { ts: 256, op: 1, ip: 16 }, None, "Probe").unwrap()
    };
    for (e, n) in [(ZipTerm::real(2.0), 2), (ZipTerm::Const(3), 3), (ZipTerm::real(0.0), 0)] {
        let text = map(pow(e.clone()));
        assert_eq!(text.matches(&format!(" ** {n})")).count(), 2, "the accelerator's term and the host's gold: {e:?}\n{text}");
        assert!(!text.contains(&format!("** {n}.")), "no power at the operand's type: {e:?}\n{text}");
    }
    let frac = map(pow(ZipTerm::real(0.5)));
    assert_eq!(frac.matches(" ** 0.5.to[Float])").count(), 2, "{frac}");
    let negative = map(pow(ZipTerm::real(-2.0)));
    assert_eq!(negative.matches(" ** (-2.0).to[Float])").count(), 2, "Spatial's integer power is for whole n >= 0\n{negative}");

    let col = [ZipColumn { name: "a".into(), fill: Fill::Pattern { modulus: 5, shift: 0 } }];
    let sum = emit_spatial_zip_sum_test(256, &col, &pow(ZipTerm::Const(2)), ZipPar { ts: 64, ip: 16, op: 1 }, "Probe").unwrap();
    assert!(sum.contains(" ** 2") && !sum.contains("** 2.to["), "{sum}");
}

/// `twins.py run` reports a run as the ledger would take it: PASS only when both networks' simulations completed —
/// a cycle count, no deadlock — and printed PASS: true; otherwise the first network that did not, and how. A run whose
/// routed network threw (BlackScholes_0: `no route for packet` on cycle 166) printed PASS: true and was reported PASS.
#[test]
fn a_twin_run_is_reported_as_the_ledger_would_take_it() {
    let work = tempfile::tempdir().unwrap();
    let out = std::process::Command::new("python3")
        .arg("-c")
        .arg(
            r#"
import importlib.util, os, sys
spec = importlib.util.spec_from_file_location("twins", sys.argv[1])
t = importlib.util.module_from_spec(spec)
spec.loader.exec_module(t)
work = sys.argv[2]
ok = "Simulation complete at cycle 6030\nPASS: true (x)\n"
cases = {
    "clean": (ok, "Simulation complete at cycle 6056\nPASS: true (x)\n"),
    "threw": (ok, "\x1b[0;31mCaught exception on cycle 166: /DUT/Top/net/router40: no route for packet 343\x1b[0m\nPASS: true (x)\nComplete Simulation\n"),
    "stuck": (ok, "Simulation complete at cycle 8853 \x1b[0;31m(DEADLOCK)\x1b[0m\nPASS: false (x)\n"),
    "wrong": (ok, "Simulation complete at cycle 9000\nPASS: false (x)\n"),
    "ideal": ("Simulation complete at cycle 7000\nPASS: false (x)\n", "Simulation complete at cycle 6056\nPASS: true (x)\n"),
    "none": None,
}
for name, logs in cases.items():
    d = os.path.join(work, name, "log")
    os.makedirs(d)
    if logs:
        open(os.path.join(d, "runp2p.log"), "w").write(logs[0])
        open(os.path.join(d, "runhybrid.log"), "w").write(logs[1])
    print(name, "|", t.run_verdict(os.path.join(work, name)))
"#,
        )
        .arg(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/plasticine/twins.py"))
        .arg(work.path())
        .output()
        .expect("python3 runs");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        text.lines().collect::<Vec<_>>(),
        [
            "clean | PASS",
            "threw | hybrid network: no completion",
            "stuck | hybrid network: deadlock at cycle 8853",
            "wrong | hybrid network: gold check failed",
            "ideal | ideal network: gold check failed",
            "none | ideal network: no log",
        ]
    );
}
