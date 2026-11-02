//! ★ RATCHET — server-side rendering scales with the page, not with its square. ★
//!
//! A server renders a page per request, so the cost that matters is the MARGINAL one: what a row
//! adds. A framework that is quietly quadratic looks fine on a fixture and falls over on the
//! first real list.
//!
//! DESIGNED FOR A LOADED MACHINE. Every number here is a RATIO between measurements taken in the
//! same process, moments apart, so a busy box slows both sides and the ratio survives. An
//! absolute pages-per-second floor would report the machine rather than the code — the lesson
//! from a timing test in this repo that fails under a 15,000-test parallel run and passes alone.
//!
//! The renders are also driven from INSIDE one program, so the parse and the prelude load are
//! paid once, the way a server pays them at startup rather than per request.

use logicaffeine_compile::compile::tw_outcome;
use std::time::Instant;

/// A page that renders `rows` list items, `renders` times.
fn page(rows: usize, renders: usize) -> String {
    let items: Vec<String> = (0..rows).map(|i| format!("\"row {i}\"")).collect();
    format!(
        "## Component App\n\
         State:\n    Let xs be [{}].\n\
         View:\n    <ul>{{for each x in xs:}}<li class=\"row\">{{x}}</li>{{end}}</ul>\n\n\
         ## Main\n\
         Let mutable total be 0.\n\
         For each i from 1 to {renders}:\n    \
         Set total to total + length of html of App.\n\
         Show total.\n",
        items.join(", ")
    )
}

/// Seconds to run `src`, asserting it ran cleanly — the BEST of several samples.
///
/// ★ THE MINIMUM, NOT A SINGLE SHOT, and the reason is that load can only ever ADD time. This
/// machine runs several sessions and a full suite alongside these tests, so one timing is a
/// measurement of the SCHEDULER as much as of the renderer, and a single unlucky sample fails a
/// ratio for a reason that has nothing to do with the code. The minimum of a few runs is the
/// standard estimator for "how fast can this go" and it is immune to noise in exactly the
/// direction the noise goes.
///
/// ★ SEVEN, NOT THREE, AND THE SMALL MEASUREMENT IS WHY. Three was not enough inside a full
/// parallel suite, and the reason is that these ratios divide a LONG measurement by a SHORT one: a
/// fixed scheduler delay is a rounding error on 200 renders and a large fraction of 1, so noise
/// pushes the denominator up and the ratio down. The short measurement is the one that needs the
/// samples, and since the helper is shared they all get them.
///
/// ★ IT COSTS NOTHING IN SENSITIVITY, which is the part that matters for a ratchet: a renderer
/// that genuinely got slower is slower in EVERY sample, so its minimum moves too and the ratios
/// below fail exactly as they did. What goes away is only the false alarm.
///
/// Found the honest way — `the_measurement_is_of_rendering_and_not_of_parsing` failed inside a
/// loaded full-suite run and passed on its own, in two different sessions on the same day. A
/// ratchet that cries wolf under load is worse than no ratchet, because the next real regression
/// is read as more noise.
fn once(src: &str) -> f64 {
    let t = Instant::now();
    let out = tw_outcome(src);
    let e = t.elapsed().as_secs_f64();
    assert_eq!(out.error, None, "the benchmark program must run cleanly");
    e
}

/// Best-of-7 for TWO programs, sampled ALTERNATELY.
///
/// ★ TAKING ALL OF ONE AND THEN ALL OF THE OTHER COMPARES TWO DIFFERENT MACHINES. Three sessions
/// share this box and a full parallel suite runs for twenty minutes, so load moves a long way
/// between the start of one seven-sample block and the end of the next. This file has now
/// produced a false red twice from exactly that: most recently "200 renders (1.901s) barely cost
/// more than 20 (2.327s)" — the small program simply got the worse slice of the machine.
///
/// Best-of-N already discards a sample that was interrupted; what it cannot fix is a machine that
/// is uniformly slower for the whole of one block. Alternating makes both programs see the same
/// conditions, so contention lands on both sides and cancels the way the fixed parse cost does.
///
/// The claims are unchanged and still strong: if a render were free, ten times as many would cost
/// the same under any load, and these ratchets would fail.
fn seconds_pair(a: &str, b: &str) -> (f64, f64) {
    let (mut best_a, mut best_b) = (f64::MAX, f64::MAX);
    for _ in 0..7 {
        best_a = best_a.min(once(a));
        best_b = best_b.min(once(b));
    }
    (best_a, best_b)
}

#[test]
fn rendering_a_page_is_linear_in_its_rows() {
    // The property a quadratic renderer fails and a fixture never notices. Ten times the rows
    // must not cost anything like a hundred times the work.
    let renders = 20;
    let (small, large) = seconds_pair(&page(10, renders), &page(100, renders));
    // Both measurements carry the SAME fixed parse cost, so the ratio understates the true
    // marginal growth — which is exactly the direction that keeps this honest: it can only fail
    // when the marginal cost is genuinely large.
    assert!(
        large < small * 12.0,
        "10x the rows cost {:.1}x the time ({small:.3}s -> {large:.3}s). Linear is the \
         requirement; this is the shape of a renderer that walks the page again per row.",
        large / small
    );
}

#[test]
fn rendering_the_same_page_twice_costs_twice_and_not_more() {
    // Per-render cost must be FLAT. A renderer that accumulated state — a growing buffer never
    // cleared, a list appended to across requests — shows up here as superlinear, and on a real
    // server as an instance that gets slower the longer it runs.
    let rows = 50;
    let (few, many) = seconds_pair(&page(rows, 10), &page(rows, 100));
    assert!(
        many < few * 12.0,
        "10x the renders cost {:.1}x the time ({few:.3}s -> {many:.3}s). Each render must cost \
         the same as the last; this is the shape of a server that degrades while it runs.",
        many / few
    );
}

#[test]
fn a_render_produces_the_bytes_it_should() {
    // A performance ratchet that measured a renderer producing NOTHING would pass forever. This
    // is the guard on the other side: the timed programs really render the page.
    let out = tw_outcome(&page(10, 1));
    assert_eq!(out.error, None);
    let total: usize = out
        .output
        .trim()
        .parse()
        .expect("the program shows a byte count");
    assert!(
        total > 100,
        "a 10-row page should be over 100 bytes of HTML, got {total}"
    );
}

#[test]
fn the_measurement_is_of_rendering_and_not_of_parsing() {
    // The honesty check on the harness itself. If parse dominated so completely that render time
    // were invisible, both ratios above would pass no matter how bad the renderer got. This
    // asserts the renders ARE a visible share.
    //
    // ★ IT COMPARES TWENTY RENDERS WITH TWO HUNDRED, NOT ONE WITH TWO HUNDRED, and that is what
    // makes it sound rather than merely lucky. Both programs pay the SAME fixed cost — the same
    // source, the same parse, the same process start — so that cost cancels and what is left is
    // the renders. The old form divided a long measurement by a very short one, and a scheduler
    // delay that is a rounding error on the long one is most of the short one: inside a full
    // parallel suite it measured `once` at 2.441s and `often` at 1.861s, reporting that two
    // hundred renders cost LESS than one. That is not a signal about the renderer at all.
    //
    // The claim is unchanged and still strong: if a render were free, doing ten times as many
    // would cost the same, and this would fail.
    let rows = 100;
    let (few, often) = seconds_pair(&page(rows, 20), &page(rows, 200));
    assert!(
        often > few * 1.15,
        "200 renders ({often:.3}s) barely cost more than 20 ({few:.3}s), so these ratchets are \
         measuring the parser, not the renderer. Raise the render count until rendering shows."
    );
}
