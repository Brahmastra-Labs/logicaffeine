//! `largo bringup` — a model on every chip from its one program.
//!
//! The checkpoint's spec is written once as a LOGOS program (`model::author`), and each backend lowers a cut of that
//! program (`logicaffeine_compile::bringup`). This prints the program's identity, then each backend's verdict: the cut
//! behind every artifact it wrote, or the name of what it does not run. With `--run`, each chip that brought the model
//! up then runs it here — the prompt, or with `--golden` and `--prompts` every prompt of a golden, each from an empty
//! state — and its verdict gains a `ran:` line — its tokens, its tokens a second and how many of them lead the
//! reference's, and a `sequence` line a prompt when there are several — or says why it did not run.

use std::path::PathBuf;

use logicaffeine_compile::bringup::{agreeing, bring_up, run_sequences, Reference, Sequence};

use crate::ui::{self, CliError};

/// What `--run` asks for: the tokens to generate, the prompt, the golden to judge against, and how many of its prompts.
pub(crate) struct RunArgs {
    pub tokens: Option<usize>,
    pub prompt: Vec<u32>,
    pub golden: Option<PathBuf>,
    pub prompts: Option<String>,
}

/// The golden's sequences `--prompts` asks for: its first, its first N, or every one.
fn chosen(mut sequences: Vec<Sequence>, prompts: Option<&str>) -> Result<Vec<Sequence>, CliError> {
    let keep = match prompts {
        None => 1,
        Some("all") => sequences.len(),
        Some(n) => match n.parse::<usize>() {
            Ok(n) if n >= 1 && n <= sequences.len() => n,
            _ => return Err(CliError::new(format!("--prompts takes `all` or 1 to {}, the golden's prompts; not `{n}`", sequences.len()))),
        },
    };
    sequences.truncate(keep);
    Ok(sequences)
}

pub(crate) fn cmd_bringup(checkpoint: PathBuf, targets: Vec<String>, ctx: usize, output: PathBuf, run_args: Option<RunArgs>) -> Result<(), Box<dyn std::error::Error>> {
    let b = bring_up(&checkpoint, ctx, &output, &targets).map_err(|e| CliError::new(e.to_string()))?;
    let runs = match run_args {
        None => None,
        Some(a) => {
            let (mut sequences, what) = match &a.golden {
                Some(path) => {
                    let (sequences, what) = Reference::golden_sequences(path).map_err(|e| CliError::new(e.to_string()))?;
                    (chosen(sequences, a.prompts.as_deref())?, what)
                }
                None => {
                    let reference = Reference::of(&b.spec, &checkpoint, &a.prompt, a.tokens.unwrap_or(8)).map_err(|e| CliError::new(e.to_string()))?;
                    (vec![Sequence { prompt: a.prompt.clone(), tokens: reference.tokens }], reference.what)
                }
            };
            if let Some(n) = a.tokens {
                for s in &mut sequences {
                    s.tokens.truncate(n);
                }
            }
            Some((run_sequences(&b, &checkpoint, ctx, &output, &sequences), what))
        }
    };
    println!("{}: {}, {} layers", checkpoint.display(), b.spec.arch, b.spec.layers.len());
    println!("{}", b.decode.line());
    println!("  the program: {}", output.join("program.lg").display());
    let ids = |t: &[u32]| t.iter().map(u32::to_string).collect::<Vec<_>>().join(" ");
    if let Some((runs, what)) = &runs {
        match runs.sequences.as_slice() {
            [one] => println!("  runs: after the prompt [{}], judged against {what}", ids(&one.prompt)),
            many => println!("  runs: {} prompts of {what}, each from an empty state and judged against its own continuation", many.len()),
        }
    }
    let mut brought = 0;
    for (chip, result) in &b.results {
        match result {
            Ok(r) => {
                brought += 1;
                println!("{chip}: {} program{}, {} files under {}", r.programs.len(), if r.programs.len() == 1 { "" } else { "s" }, r.files.len(), output.join(chip).display());
                for p in &r.programs {
                    println!("  {}", p.line());
                }
            }
            Err(why) => println!("{chip}: refused: {why}"),
        }
        let Some((runs, _)) = &runs else { continue };
        match runs.results.iter().find(|(c, _)| c == chip).map(|(_, r)| r) {
            Some(Ok(g)) => {
                let agree: Vec<usize> = g.tokens.iter().zip(&runs.sequences).map(|(t, s)| agreeing(t, &s.tokens)).collect();
                let of: usize = runs.sequences.iter().map(|s| s.tokens.len()).sum();
                match g.tokens.as_slice() {
                    [one] => println!(
                        "  ran: {} tokens [{}] on {}, {:.2} tok/s, {} of {of} agree with the reference",
                        one.len(),
                        ids(one),
                        g.on,
                        g.tok_s(),
                        agree[0]
                    ),
                    many => {
                        println!(
                            "  ran: {} sequences, {} tokens on {}, {:.2} tok/s, {} of {of} agree with the reference",
                            many.len(),
                            many.iter().map(Vec::len).sum::<usize>(),
                            g.on,
                            g.tok_s(),
                            agree.iter().sum::<usize>()
                        );
                        for (i, ((t, s), a)) in many.iter().zip(&runs.sequences).zip(&agree).enumerate() {
                            println!("    sequence {}: [{}], {a} of {} agree", i + 1, ids(t), s.tokens.len());
                        }
                    }
                }
                println!("  the program it ran: {}", g.program.display());
            }
            Some(Err(why)) => println!("  did not run: {why}"),
            None => {}
        }
    }
    if brought == 0 {
        return Err(CliError::new("no backend brought the model up").into());
    }
    ui::info(format!("Brought {} up on {brought} of {} backends", checkpoint.display(), b.results.len()));
    Ok(())
}
