//! Latency locks: analysis of a large document must stay interactive.
//!
//! Ceilings are deliberately generous (an order of magnitude over today's
//! numbers) — they exist to fail LOUDLY on catastrophic regressions
//! (accidental O(n²) passes, a synchronous cargo call on the keystroke
//! path), not to flake on a busy CI box. If one of these fires, the block
//! cache from the campaign plan (§A7) is the designed next step.

use std::time::{Duration, Instant};

use logicaffeine_lsp::document::DocumentState;
use logicaffeine_lsp::semantic_tokens::encode_document_tokens;

/// ~2k lines: 40 functions and a Main that exercises bindings, loops,
/// arithmetic, collections, and calls — the shape of a real module.
fn large_corpus() -> String {
    let mut source = String::new();
    for i in 0..40 {
        source.push_str(&format!(
            "## To helper{i} (n: Int) -> Int:\n    \
             Let doubled be n * 2.\n    \
             Let mutable acc be 0.\n    \
             While acc is less than doubled:\n        \
             Set acc to acc + 1.\n    \
             Return acc.\n\n"
        ));
    }
    source.push_str("## Main\n");
    source.push_str("Let mutable total be 0.\n");
    for i in 0..40 {
        source.push_str(&format!("Let r{i} be helper{i}({i}).\n"));
        source.push_str(&format!("Set total to total + r{i}.\n"));
    }
    for i in 0..600 {
        source.push_str(&format!("Let v{i} be {i} + 1.\n"));
        source.push_str(&format!("Set total to total + v{i}.\n"));
    }
    source.push_str("Show total.\n");
    source
}

#[test]
fn full_analysis_of_a_large_document_stays_interactive() {
    let source = large_corpus();
    assert!(
        source.lines().count() > 1500,
        "the corpus must actually be large: {} lines",
        source.lines().count()
    );

    let start = Instant::now();
    let doc = DocumentState::new(source, 1);
    let elapsed = start.elapsed();

    assert!(
        !doc.symbol_index.definitions.is_empty(),
        "the analysis must have actually run"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "analyzing ~2k lines took {elapsed:?} — over the 5s catastrophe ceiling; \
         time for the block-level reparse cache (plan §A7)"
    );
}

#[test]
fn semantic_token_encoding_of_a_large_document_stays_interactive() {
    let doc = DocumentState::new(large_corpus(), 1);

    let start = Instant::now();
    let tokens = encode_document_tokens(&doc);
    let elapsed = start.elapsed();

    assert!(tokens.len() > 2_000, "a large doc has thousands of tokens");
    assert!(
        elapsed < Duration::from_secs(2),
        "token encoding took {elapsed:?} — over the 2s catastrophe ceiling"
    );
}
