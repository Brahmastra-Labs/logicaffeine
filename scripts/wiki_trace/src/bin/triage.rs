//! Triage an English text file through the LOGOS English→FOL compiler: classify
//! every sentence into the kind of work it implies, localize it, derive an oracle
//! where a paraphrase already parses, and emit an actionable worklist + a
//! machine-readable record set for an autonomous improvement loop.
//!
//! READ-ONLY: writes only under the output directory. Never edits source,
//! lexicon, or tests — it proposes; the loop (gated) applies.
//!
//! Usage: wiki-triage <input.txt> [output_dir]
//! Default output: wikis/triage/<input-stem>/

use logicaffeine_compile::ui_bridge::compile_for_ui;
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::process::exit;
use wiki_trace::{
    classify, cluster, render::render_trace, Category, Gate, Outcome, TriageRecord,
};

fn main() {
    let mut args = std::env::args().skip(1);
    let input = match args.next() {
        Some(p) => PathBuf::from(p),
        None => {
            eprintln!("usage: wiki-triage <input.txt> [output_dir]");
            exit(2);
        }
    };

    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "article".to_string());

    let out_dir = match args.next() {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from("wikis").join("triage").join(&stem),
    };

    let text = match fs::read_to_string(&input) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("cannot read {}: {e}", input.display());
            exit(1);
        }
    };

    let sentences: Vec<(usize, String)> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|s| s.to_string())
        .enumerate()
        .map(|(i, s)| (i + 1, s))
        .collect();

    let sent_dir = out_dir.join("sentences");
    if let Err(e) = fs::create_dir_all(&sent_dir) {
        eprintln!("cannot create {}: {e}", sent_dir.display());
        exit(1);
    }

    let mut records: Vec<TriageRecord> = Vec::new();
    let mut jsonl = String::new();

    for (n, sentence) in &sentences {
        let record = classify(*n, sentence);

        // Enriched per-sentence dump: classification header + the verbose trace.
        let result = compile_for_ui(sentence);
        let mut dump = String::new();
        dump.push_str(&format!(
            "CLASSIFICATION: {:?} | subsystem {:?} | gate {:?} | confidence {:.2}\n",
            record.category, record.subsystem, record.gate, record.confidence
        ));
        if let Some(o) = &record.oracle {
            dump.push_str(&format!(
                "ORACLE [{}]: {:?} ⇒ {}\n",
                o.transform, o.variant_sentence, o.expected_fol
            ));
        }
        if !record.localization.isolated_spans.is_empty() {
            let spans: Vec<String> = record
                .localization
                .isolated_spans
                .iter()
                .map(|s| format!("{:?}({:?})", s.text, s.kind))
                .collect();
            dump.push_str(&format!("ISOLATED: {}\n", spans.join(", ")));
        }
        dump.push('\n');
        dump.push_str(&render_trace(*n, sentence, &result));
        let _ = fs::write(sent_dir.join(format!("{n:02}.trace")), &dump);

        jsonl.push_str(&serde_json::to_string(&record).unwrap_or_default());
        jsonl.push('\n');
        records.push(record);
    }

    let clusters = cluster(&records);

    // ── verdict.json: the loop's exit condition ───────────────────────────────
    let total = records.len();
    let clean = records.iter().filter(|r| r.category == Category::Clean).count();
    let all_clean = clean == total && total > 0;
    let count_cat = |c: Category| records.iter().filter(|r| r.category == c).count();
    let count_out = |o: Outcome| records.iter().filter(|r| r.outcome == o).count();

    let verdict = json!({
        "article": input.display().to_string(),
        "all_clean": all_clean,
        "sentences": total,
        "outcomes": {
            "ok": count_out(Outcome::Ok),
            "partial": count_out(Outcome::Partial),
            "fail": count_out(Outcome::Fail),
        },
        "by_category": {
            "clean": clean,
            "actionable_lexicon_gap": count_cat(Category::ActionableLexiconGap),
            "parser_gap": count_cat(Category::ParserGap),
            "semantic_lossy": count_cat(Category::SemanticLossy),
            "ambiguity_human": count_cat(Category::AmbiguityHuman),
            "isolate_out_of_scope": count_cat(Category::IsolateOutOfScope),
        },
        "clusters": clusters.len(),
    });

    let _ = fs::write(
        out_dir.join("verdict.json"),
        serde_json::to_string_pretty(&verdict).unwrap_or_default(),
    );
    let _ = fs::write(out_dir.join("triage.jsonl"), &jsonl);
    let _ = fs::write(
        out_dir.join("clusters.json"),
        serde_json::to_string_pretty(&clusters).unwrap_or_default(),
    );
    let _ = fs::write(out_dir.join("worklist.md"), worklist_md(&input, &records, &clusters));
    let _ = fs::write(out_dir.join("needs_human.md"), needs_human_md(&input, &records));

    println!(
        "{} sentences → {} clean, {} lexicon, {} parser, {} semantic, {} human, {} isolate",
        total,
        clean,
        count_cat(Category::ActionableLexiconGap),
        count_cat(Category::ParserGap),
        count_cat(Category::SemanticLossy),
        count_cat(Category::AmbiguityHuman),
        count_cat(Category::IsolateOutOfScope),
    );
    println!(
        "verdict: {}",
        if all_clean {
            "ALL CLEAN ✓ — article compiles to proper FOL"
        } else {
            "work remaining — see worklist.md"
        }
    );
    println!("triage written to {}/", out_dir.display());
}

fn worklist_md(
    input: &std::path::Path,
    records: &[TriageRecord],
    clusters: &[wiki_trace::Cluster],
) -> String {
    let mut s = String::new();
    s.push_str(&format!("# Worklist — {}\n\n", input.display()));
    s.push_str(
        "Actionable items only (gate `auto` or `investigate`). Design decisions and \
         isolated noise are in `needs_human.md`. Fix a *cluster*, not a single line.\n\n",
    );

    s.push_str("## Clusters (fix the class)\n\n");
    let actionable: Vec<_> = clusters
        .iter()
        .filter(|c| c.gate != Gate::Human)
        .collect();
    if actionable.is_empty() {
        s.push_str("_None._\n\n");
    } else {
        for c in actionable {
            s.push_str(&format!(
                "- **{}** ×{} — {:?}/{:?}, gate `{:?}` — e.g. {:?} (sentences {:?})\n",
                c.signature, c.count, c.category, c.subsystem, c.gate, c.example, c.members
            ));
        }
        s.push('\n');
    }

    for (title, gate) in [("## Auto-eligible (lexicon, low risk)", Gate::Auto),
                          ("## Investigate (agent + human judgment)", Gate::Investigate)] {
        s.push_str(title);
        s.push_str("\n\n");
        let mut items: Vec<&TriageRecord> = records.iter().filter(|r| r.gate == gate).collect();
        items.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());
        if items.is_empty() {
            s.push_str("_None._\n\n");
            continue;
        }
        for r in items {
            s.push_str(&format!(
                "### {:02}. {:?} ({:.2})\n",
                r.index, r.category, r.confidence
            ));
            s.push_str(&format!("- input: {:?}\n", r.input));
            if !r.localization.error_kind.is_empty() {
                s.push_str(&format!(
                    "- error: `{}`{}\n",
                    r.localization.error_kind,
                    r.localization
                        .offending_text
                        .as_ref()
                        .map(|t| format!(" at {t:?}"))
                        .unwrap_or_default()
                ));
            }
            if !r.localization.suspect_words.is_empty() {
                s.push_str(&format!("- suspect words: {:?}\n", r.localization.suspect_words));
            }
            if let Some(o) = &r.oracle {
                s.push_str(&format!(
                    "- oracle [{}]: paraphrase {:?} parses ⇒ expected `{}`\n",
                    o.transform, o.variant_sentence, o.expected_fol
                ));
            }
            if let Some(e) = &r.proposal.lexicon_entry {
                s.push_str(&format!(
                    "- proposed lexicon entry: `{}` as `{}` ({})\n",
                    e.lemma, e.pos, e.note
                ));
            }
            if let Some(t) = &r.proposal.red_test {
                s.push_str("- proposed RED test:\n```rust\n");
                s.push_str(t);
                s.push_str("```\n");
            }
            s.push('\n');
        }
    }
    s
}

fn needs_human_md(input: &std::path::Path, records: &[TriageRecord]) -> String {
    let mut s = String::new();
    s.push_str(&format!("# Needs human — {}\n\n", input.display()));
    s.push_str(
        "Design decisions, genuine ambiguity, semantic conflicts, and isolated noise \
         (abbreviations, parentheticals, quotes, citations). The loop never acts on these \
         autonomously.\n\n",
    );
    let items: Vec<&TriageRecord> = records.iter().filter(|r| r.gate == Gate::Human).collect();
    if items.is_empty() {
        s.push_str("_None._\n");
        return s;
    }
    for r in items {
        s.push_str(&format!("### {:02}. {:?}\n", r.index, r.category));
        s.push_str(&format!("- input: {:?}\n", r.input));
        if r.category == Category::IsolateOutOfScope && !r.localization.isolated_spans.is_empty() {
            let spans: Vec<String> = r
                .localization
                .isolated_spans
                .iter()
                .map(|sp| format!("{:?}({:?})", sp.text, sp.kind))
                .collect();
            s.push_str(&format!("- isolated: {}\n", spans.join(", ")));
        }
        if !r.localization.error_kind.is_empty() {
            s.push_str(&format!("- error: `{}`\n", r.localization.error_kind));
        }
        s.push('\n');
    }
    s
}
