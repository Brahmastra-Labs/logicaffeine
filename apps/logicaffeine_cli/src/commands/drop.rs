//! `largo drop` and `largo check-drop` — a self-checking SN50 artifact drop (`logicaffeine_compile::spatial::drop`).
//!
//! `largo drop` writes the decode bundle, the checkpoint whose weights it binds (or its digest, with
//! `--fetch-weights`), the goldens of a prompt our spatial evaluator decodes, and `drop.json`. `largo check-drop` is the
//! runner: every digest, every program's certificate, every golden decoded again bit for bit, or a refusal naming what
//! failed.

use std::path::PathBuf;

use logicaffeine_compile::spatial::drop::{check_drop, write_drop};

use crate::ui::{self, CliError};

pub(crate) fn cmd_drop(checkpoint: PathBuf, ctx: usize, prompt: Vec<u32>, tokens: usize, fetch_weights: bool, output: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    if ctx == 0 || prompt.is_empty() || prompt.len() + tokens > ctx {
        return Err(CliError::with_hint(
            format!("a prompt of {} tokens and {tokens} more in a {ctx}-position cache", prompt.len()),
            "pass a non-empty --prompt whose tokens and --tokens fit --ctx",
        )
        .exit_code(ui::EXIT_USAGE)
        .into());
    }
    let drop = write_drop(&checkpoint, ctx, &[prompt], tokens, fetch_weights, &output).map_err(|e| CliError::new(e.to_string()))?;
    let files = drop["files"].as_object().map_or(0, |f| f.len());
    println!("dropped {} for the {} at {ctx} positions: {files} files, digested in {}", drop["model"].as_str().unwrap_or("the model"), drop["chip"].as_str().unwrap_or("sn50"), output.join("drop.json").display());
    for g in drop["goldens"].as_array().into_iter().flatten() {
        println!("golden: prompt {} decodes {} (the f64 reference: {})", g["prompt"], g["tokens"], g["reference_tokens"]);
    }
    Ok(())
}

pub(crate) fn cmd_check_drop(dir: PathBuf, weights: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let v = check_drop(&dir, weights.as_deref()).map_err(|e| CliError::new(e.to_string()))?;
    println!(
        "verified {}: {} files by sha256, {} programs' certificates rechecked, {} goldens decoded again bit for bit",
        dir.display(),
        v.files,
        v.certificates,
        v.goldens
    );
    Ok(())
}
