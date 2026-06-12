//! Compile an English text file through the LOGOS English→FOL compiler and dump
//! verbose per-sentence traces (tokens, AST, FOL×3, ambiguity readings, errors).
//!
//! Usage: wiki-trace <input.txt> [output_dir]
//!
//! Output (default wikis/traces/<input-stem>/):
//!   summary.txt        one-line status of every sentence
//!   traces.jsonl       one full CompileResult per line
//!   sentences/NN.trace verbose dump per sentence

use logicaffeine_compile::ui_bridge::compile_for_ui;
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::process::exit;
use wiki_trace::{count_nodes, render::render_trace};

fn main() {
    let mut args = std::env::args().skip(1);
    let input = match args.next() {
        Some(p) => PathBuf::from(p),
        None => {
            eprintln!("usage: wiki-trace <input.txt> [output_dir]");
            exit(2);
        }
    };

    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "article".to_string());

    let out_dir = match args.next() {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from("wikis").join("traces").join(&stem),
    };

    let text = match fs::read_to_string(&input) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("cannot read {}: {e}", input.display());
            exit(1);
        }
    };

    let sentences: Vec<(usize, &str)> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .enumerate()
        .collect();

    let sent_dir = out_dir.join("sentences");
    if let Err(e) = fs::create_dir_all(&sent_dir) {
        eprintln!("cannot create {}: {e}", sent_dir.display());
        exit(1);
    }

    let mut jsonl = String::new();
    let mut summary = String::new();
    summary.push_str(&format!("# Trace map for {}\n", input.display()));
    summary.push_str(&format!("# {} sentences\n\n", sentences.len()));

    let mut ok = 0usize;
    let mut errored = 0usize;
    let mut partial = 0usize;

    for (i, sentence) in &sentences {
        let n = i + 1;
        let result = compile_for_ui(sentence);
        let (other_nodes, total_nodes) = result.ast.as_ref().map(count_nodes).unwrap_or((0, 0));

        let status = if result.error.is_some() {
            errored += 1;
            " FAIL "
        } else if other_nodes > 0 {
            partial += 1;
            " PART "
        } else {
            ok += 1;
            "  ok  "
        };

        let trace = render_trace(n, sentence, &result);
        let _ = fs::write(sent_dir.join(format!("{n:02}.trace")), &trace);

        summary.push_str(&format!("[{status}] {n:02}. {sentence}\n"));
        if let Some(err) = &result.error {
            summary.push_str(&format!("           ↳ {}\n", err.replace('\n', "\n           ")));
        } else {
            summary.push_str(&format!("           ⊢ {}\n", result.logic.as_deref().unwrap_or("")));
            if other_nodes > 0 {
                summary.push_str(&format!(
                    "           ⚠ {other_nodes}/{total_nodes} AST nodes are unhandled variants\n"
                ));
            }
        }
        summary.push('\n');

        let full = json!({
            "index": n,
            "input": sentence,
            "result": serde_json::to_value(&result).unwrap_or(serde_json::Value::Null),
        });
        jsonl.push_str(&full.to_string());
        jsonl.push('\n');
    }

    summary.push_str(&format!(
        "# totals: {ok} ok, {partial} partial, {errored} error (of {})\n",
        sentences.len()
    ));

    let _ = fs::write(out_dir.join("summary.txt"), &summary);
    let _ = fs::write(out_dir.join("traces.jsonl"), &jsonl);

    println!(
        "{} sentences → {ok} ok, {partial} partial, {errored} error",
        sentences.len()
    );
    println!("traces written to {}/", out_dir.display());
}
